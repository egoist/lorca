package model

import (
	"encoding/json"
	"errors"
	"log"
	"maps"
	"slices"
	"strings"
	"time"
)

// The app's model, after the macOS app's AppStore. Everything comes from the CLI over 127.0.0.1;
// mutations are applied optimistically and confirmed by the events the CLI sends back.
// `LORCA_MOCK=1` runs the seeded demo with the in-process reply engine instead.
//
// The store lives on the main thread, like the Swift one: the views read it while they build, its
// methods change it, and its state is read right after it is written. Requests go out on
// goroutines, and their answers come back to the main thread through `post` before they touch the
// store or reach a callback. Views learn what changed from its events.

// Transport is the connection to the CLI: one websocket, safe to call from any goroutine.
type Transport interface {
	// Request sends one call of the CLI's JSON API and waits for its result.
	Request(method string, params any) (json.RawMessage, error)
	// Reconnect drops the socket and looks for the CLI again.
	Reconnect()
}

// LaunchFailure is why the CLI is not running, for the offline state to word.
type LaunchFailure struct {
	// Kind is "missing_binary", "launch", "exited", or "startup_closed".
	Kind   string
	Binary string
	Reason string
	Code   int
	Log    string
}

// LauncherStatus is where the CLI's lifecycle stands.
type LauncherStatus struct {
	// Kind is "idle", "probing", "starting", "running", or "failed".
	Kind string
	// External is a CLI that was already answering on the port: the app uses it and starts none.
	External bool
	Failure  *LaunchFailure
}

// CLIState is where the connection to the CLI stands, for the loading and offline states.
type CLIState struct {
	// Connection is "disconnected", "connecting", or "connected".
	Connection string
	Launcher   LauncherStatus
	// Starting is the first connection still loading: the window shows a spinner, not the offline
	// recovery controls.
	Starting bool
}

type EventKind int

const (
	EventSnapshotReplaced EventKind = iota
	EventRosterChanged
	EventChatsChanged
	EventChatChanged
	EventMessageAdded
	EventMessageChanged
	EventMessageRemoved
	EventRespondingChanged
	// EventOlderMessagesLoaded is a page of older messages put ahead of the chat's first one.
	EventOlderMessagesLoaded
	// EventTurnFinished is a bot's turn that ended; StartedAt is when this app saw it start.
	EventTurnFinished
	// EventRunningTasksChanged is a command in the chat that has run long enough to count as a
	// running task.
	EventRunningTasksChanged
	// EventOutputsChanged is the chat's published outputs that changed.
	EventOutputsChanged
	EventConnectionChanged
	EventIdentityChanged
	// EventFeedbackChanged is a bot's workflow feedback changing on its Runner (BotID).
	EventFeedbackChanged
	EventBudgetsChanged
	EventReviewsChanged
	EventDurableTasksChanged
	EventAttentionChanged
	EventProjectContextChanged
)

// Event says what in the store changed.
type Event struct {
	Kind      EventKind
	ChatID    string
	MessageID string
	BotID     string
	StartedAt time.Time
}

// OutgoingAttachment is a file picked, dropped, or pasted into the composer, before the CLI stores
// it. The id is minted here so the bubble the app shows right away and the CLI's copy agree.
type OutgoingAttachment struct {
	Attachment Attachment
	Path       string
}

type runningJob struct {
	id     string
	chatID string
	// botID is empty while a group exchange is between member turns.
	botID     string
	routineID string
}

// RequestError is a request the CLI refused or could not answer, in the app's words.
type RequestError struct{ Message string }

func (e *RequestError) Error() string { return e.Message }

// ErrorText is the words of an error the app or the CLI raised, in the app's language when it has
// them.
func ErrorText(err error) string {
	if err == nil {
		return ""
	}
	return L(err.Error())
}

// TaskDelay is how long a command runs before it counts as a running task.
const TaskDelay = 2 * time.Second

type Store struct {
	// IsMock is the seeded demo (`LORCA_MOCK=1`), which runs without a CLI.
	IsMock    bool
	transport Transport
	// post runs a function on the main thread, after the current one returns.
	post func(func())

	Devices []*Device
	Bots    []*Bot
	Chats   []*Chat
	// Routines are every bot's routines, from the roster.
	Routines     []*Routine
	DurableTasks []*DurableTask
	// Reviews are read-only CLI projections; the owning Runner decides and executes.
	Reviews []*ReviewItem
	// Budgets mirror Runner-owned usage and recovery state; edits go through the local CLI.
	Budgets []BudgetState
	// AutoReview is shared through the roster.
	AutoReview AutoReview
	// Attention is what waits on the user across chats, kept by the bots (attention.changed).
	Attention AttentionView
	// SharedLinks are the bots the account shares as links, shared through the roster.
	SharedLinks []SharedLink
	// Providers are the account's provider credentials, the same on every Device.
	Providers []ProviderCredential
	// Models are what the CLI's catalog offers, for the Model and Thinking pickers.
	Models []ProviderModel
	// Playbooks are every bot's and group's skills and drafts, from the roster; a body is fetched
	// when one opens.
	Playbooks []PlaybookSummary
	// mockPlaybooks are the demo's skills, bodies and all.
	mockPlaybooks []PlaybookRecord

	// IsConnected is the CLI answering on localhost (mock: toggled from the Debug menu).
	IsConnected bool
	// IsStarting is the first connection still loading.
	IsStarting bool
	// HasIdentity is nil until the CLI has supplied the initial snapshot.
	HasIdentity      *bool
	IsIdentityDevice bool
	IdentityID       string
	RelayConnected   bool
	// RelayUpdateRequired is the relay refusing this build's protocol: it syncs again once Lorca
	// is updated.
	RelayUpdateRequired bool
	// RelayError is why the last try to connect to the relay failed, as the CLI words it.
	RelayError string
	RelayURL   string
	// CLI is where the connection and the launcher stand, for the offline state.
	CLI CLIState

	// runningJobs are turns in flight, by job id: the chat and the bot, and the routine when the
	// turn is one of its runs.
	runningJobs []runningJob
	// jobStarts is when each turn in flight was first seen.
	jobStarts map[string]time.Time
	// commandStarts is when this app saw each command start running in its terminal, by row.
	commandStarts map[string]time.Time
	// reportedWatchedChat is the chat last reported to the CLI as on screen.
	reportedWatchedChat *string
	loadingOlder        map[string]bool
	replies             *replyEngine
	started             bool
	startupTimer        *time.Timer
	bootstrapGeneration int
	isBootstrapping     bool
	bootstrapEvents     []pendingEvent
	isApplyingBootstrap bool
	listeners           []func(Event)

	// retryNotes is "Retrying (2 of 3) in 4 s", while a turn's model call waits to be asked again.
	retryNotes map[string]string
	// thinkingBots is the bot whose model is reasoning in a chat.
	thinkingBots map[string]string

	// attachmentFiles is where each attachment's bytes are on this computer.
	attachmentFiles    map[string]string
	fetchingAttachment map[string]bool
	// attachmentErrors is why a fetch failed, kept until a retry so a scroll does not ask again.
	attachmentErrors map[string]string
	// outputMessages is every version of each shown chat's outputs, oldest first; outputRequests
	// are the chats whose list is on its way.
	outputMessages map[string][]*Message
	outputRequests map[string]bool
	staleOutputs   map[string]bool

	mockWorkflows        map[string]*demoSetup
	mockWorkflowAccounts map[string][]WorkflowAccount
	mockMarketplace      *Marketplace
	mockMcp              map[string][]McpServer
	// mockFeedback is the demo's workflow feedback, changed in place by the same calls.
	mockFeedback map[string]BotFeedback
	mockBrowser  map[string][]BrowserProfile
}

type pendingEvent struct {
	name string
	data json.RawMessage
}

// NewStore makes the store. `post` runs a function on the main thread.
func NewStore(transport Transport, post func(func()), mock bool) *Store {
	return &Store{
		IsMock:             mock,
		transport:          transport,
		post:               post,
		IsStarting:         true,
		AutoReview:         AutoReview{IsEnabled: true},
		Attention:          DefaultAttention(),
		CLI:                CLIState{Connection: "disconnected", Launcher: LauncherStatus{Kind: "idle"}, Starting: true},
		jobStarts:          map[string]time.Time{},
		commandStarts:      map[string]time.Time{},
		loadingOlder:       map[string]bool{},
		retryNotes:         map[string]string{},
		thinkingBots:       map[string]string{},
		attachmentFiles:    map[string]string{},
		fetchingAttachment: map[string]bool{},
		attachmentErrors:   map[string]string{},
		outputMessages:     map[string][]*Message{},
		outputRequests:     map[string]bool{},
		staleOutputs:       map[string]bool{},
		mockMcp:            map[string][]McpServer{},
		mockFeedback:       map[string]BotFeedback{},
		isBootstrapping:    true,
	}
}

// Async runs work on a goroutine and hands its result to done on the main thread.
func Async[T any](s *Store, work func() (T, error), done func(T, error)) {
	go func() {
		value, err := work()
		s.post(func() {
			if done != nil {
				done(value, err)
			}
		})
	}()
}

// later calls fn on the main thread after d.
func (s *Store) later(d time.Duration, fn func()) *time.Timer {
	return time.AfterFunc(d, func() { s.post(fn) })
}

// MARK: - Lifecycle

// Start loads the demo, or waits for the CLI: the app calls CLIStateChanged and HandleEvent from
// here on.
func (s *Store) Start() {
	if s.started {
		return
	}
	s.started = true
	if s.IsMock {
		s.replies = newReplyEngine(s)
		s.ResetMockData()
		s.IsConnected = true
		s.finishStartup()
		yes := true
		s.HasIdentity = &yes
		s.IsIdentityDevice = true
		s.emit(Event{Kind: EventConnectionChanged})
		s.emit(Event{Kind: EventIdentityChanged})
		return
	}
	// A slow or silent CLI leaves the user with the offline recovery controls. This deadline
	// bounds the loading state, never the time until a window is shown.
	s.startupTimer = s.later(2500*time.Millisecond, func() {
		s.finishStartup()
		s.emit(Event{Kind: EventConnectionChanged})
	})
}

// CLIStateChanged takes where the connection and the launcher stand now.
func (s *Store) CLIStateChanged(state CLIState) {
	if s.IsMock {
		return
	}
	wasConnected := s.CLI.Connection == "connected"
	s.CLI = state
	if state.Launcher.Kind == "failed" {
		s.finishStartup()
	}
	if state.Connection == "connected" {
		if !wasConnected {
			s.bootstrapGeneration++
			s.bootstrap(s.bootstrapGeneration)
		}
	} else {
		s.bootstrapGeneration++
		s.isBootstrapping = true
		s.bootstrapEvents = nil
		s.reportedWatchedChat = nil
		if s.IsConnected {
			s.IsConnected = false
			s.runningJobs = nil
			clear(s.thinkingBots)
		}
	}
	s.emit(Event{Kind: EventConnectionChanged})
}

func (s *Store) finishStartup() {
	if s.startupTimer != nil {
		s.startupTimer.Stop()
		s.startupTimer = nil
	}
	s.IsStarting = false
}

// Reconnect looks for the CLI again, for Try Again.
func (s *Store) Reconnect() {
	if s.IsMock {
		s.SetConnected(true)
		return
	}
	if s.transport != nil {
		go s.transport.Reconnect()
	}
}

// request calls the CLI and decodes its result into `into` (nil to drop it). It blocks: call it
// from a goroutine.
func (s *Store) request(method string, params any, into any) error {
	if s.transport == nil {
		return &RequestError{L("The Lorca CLI is not running")}
	}
	if params == nil {
		params = map[string]any{}
	}
	raw, err := s.transport.Request(method, params)
	if err != nil {
		return &RequestError{ErrorText(err)}
	}
	if into != nil && len(raw) > 0 {
		if err := json.Unmarshal(raw, into); err != nil {
			return &RequestError{err.Error()}
		}
	}
	return nil
}

// call is request for one result type, for Async.
func call[T any](s *Store, method string, params any) (T, error) {
	var value T
	err := s.request(method, params, &value)
	return value, err
}

func (s *Store) bootstrap(generation int) {
	Async(s, func() (WireSnapshot, error) {
		return call[WireSnapshot](s, "bootstrap", nil)
	}, func(snapshot WireSnapshot, err error) {
		if generation != s.bootstrapGeneration {
			return
		}
		if err != nil {
			s.finishStartup()
			s.emit(Event{Kind: EventConnectionChanged})
			log.Printf("bootstrap failed: %s", ErrorText(err))
			return
		}
		// The snapshot includes identity and connection state as well as messages. Publish them
		// together, so roster events cannot expose empty previews before it arrives.
		s.isApplyingBootstrap = true
		s.apply(snapshot)
		for _, event := range s.bootstrapEvents {
			s.handle(event.name, event.data)
		}
		s.bootstrapEvents = nil
		s.isApplyingBootstrap = false
		s.isBootstrapping = false
		s.IsConnected = true
		s.finishStartup()
		s.emit(Event{Kind: EventSnapshotReplaced})
		s.emit(Event{Kind: EventConnectionChanged})
		s.emit(Event{Kind: EventIdentityChanged})
	})
}

func (s *Store) apply(snapshot WireSnapshot) {
	previousTasks, previousIdentity := s.DurableTasks, s.IdentityID
	if next := str(snapshot.IdentityID); next != s.IdentityID {
		clear(s.attachmentFiles)
		clear(s.fetchingAttachment)
		clear(s.attachmentErrors)
	}
	// A resync may bring outputs this app missed; they are asked for again when next shown.
	for chatID := range s.outputMessages {
		s.staleOutputs[chatID] = true
	}
	has := snapshot.HasIdentity
	s.HasIdentity = &has
	s.IsIdentityDevice = snapshot.IsIdentityDevice
	s.IdentityID = str(snapshot.IdentityID)
	s.RelayURL = str(snapshot.RelayURL)
	s.RelayConnected = snapshot.RelayConnected
	s.RelayUpdateRequired = flag(snapshot.RelayUpdateRequired)
	s.RelayError = ""
	if snapshot.RelayError != nil {
		s.RelayError = snapshot.RelayError.Message
	}
	s.Devices = s.Devices[:0:0]
	for _, device := range snapshot.Devices {
		s.Devices = append(s.Devices, ToDevice(device))
	}
	s.Bots = s.Bots[:0:0]
	for _, bot := range snapshot.Bots {
		s.Bots = append(s.Bots, ToBot(bot))
	}
	// A snapshot carries each chat's newest messages. Older pages this app already loaded stay
	// ahead of them, so a resync does not throw the transcript back to the last page.
	loaded := s.Chats
	chats := make([]*Chat, 0, len(snapshot.Chats))
	for _, incoming := range snapshot.Chats {
		chat := ToChat(incoming, nil)
		if existing := findChat(loaded, chat.ID); existing != nil && len(chat.Messages) > 0 {
			first := chat.Messages[0].ID
			if index := slices.IndexFunc(existing.Messages, func(m *Message) bool { return m.ID == first }); index > 0 {
				chat.Messages = append(slices.Clone(existing.Messages[:index]), chat.Messages...)
				chat.HasMore = existing.HasMore
			}
		}
		chats = append(chats, chat)
	}
	s.Chats = chats
	s.Routines = s.Routines[:0:0]
	s.DurableTasks = nil
	for _, task := range snapshot.Tasks {
		copy := task.Clone()
		if snapshot.HasIdentity && previousIdentity == s.IdentityID {
			for _, previous := range previousTasks {
				if previous.ID == task.ID && previous.Revision > task.Revision {
					copy = previous.Clone()
					break
				}
			}
		}
		s.DurableTasks = append(s.DurableTasks, &copy)
	}
	if snapshot.HasIdentity && previousIdentity == s.IdentityID {
		// Tasks are retained records; cancellation changes state rather than deleting one.
		// A snapshot taken before a newly delivered record must not remove that record.
		for _, previous := range previousTasks {
			if s.DurableTask(previous.ID) == nil {
				copy := previous.Clone()
				s.DurableTasks = append(s.DurableTasks, &copy)
			}
		}
	}
	for _, routine := range snapshot.Routines {
		s.Routines = append(s.Routines, ToRoutine(routine))
	}
	s.AutoReview = ToAutoReview(snapshot.AutoReview)
	s.Attention = DefaultAttention()
	if snapshot.Attention != nil {
		s.Attention = *snapshot.Attention
	}
	s.Budgets = slices.Clone(snapshot.Budgets)
	s.Reviews = nil
	for _, item := range snapshot.Reviews {
		copy := item.Clone()
		s.Reviews = append(s.Reviews, &copy)
	}
	s.SharedLinks = snapshot.SharedLinks
	s.Providers = ToProviders(snapshot.Providers)
	s.Models = ToModels(snapshot.Models)
	s.Playbooks = snapshot.Playbooks
	s.runningJobs = nil
	for _, turn := range snapshot.RunningTurns {
		s.runningJobs = append(s.runningJobs, runningJob{id: turn.JobID, chatID: turn.ChatID, botID: turn.BotID, routineID: str(turn.RoutineID)})
	}
	for _, id := range snapshot.RunningChatIDs {
		if !slices.ContainsFunc(s.runningJobs, func(job runningJob) bool { return job.chatID == id }) {
			s.runningJobs = append(s.runningJobs, runningJob{id: "chat:" + id, chatID: id})
		}
	}
	s.sortChats()
	for _, chat := range s.Chats {
		for _, message := range chat.Messages {
			s.noteCommand(message, chat.ID)
		}
	}
	s.emit(Event{Kind: EventSnapshotReplaced})
}

// MARK: - Events from the CLI

// HandleEvent takes an event frame's name and data from the CLI.
func (s *Store) HandleEvent(name string, data json.RawMessage) {
	if s.IsMock {
		return
	}
	if s.isBootstrapping {
		s.bootstrapEvents = append(s.bootstrapEvents, pendingEvent{name, data})
		return
	}
	s.handle(name, data)
}

func decode[T any](data json.RawMessage) (T, bool) {
	var value T
	if err := json.Unmarshal(data, &value); err != nil {
		log.Printf("event: %v", err)
		return value, false
	}
	return value, true
}

func (s *Store) handle(name string, data json.RawMessage) {
	switch name {
	case "projects.changed":
		if payload, ok := decode[struct {
			ChatID string `json:"chat_id"`
		}](data); ok {
			s.emit(Event{Kind: EventProjectContextChanged, ChatID: payload.ChatID})
		}
	case "attention.changed":
		if view, ok := decode[AttentionView](data); ok {
			s.applyAttention(view)
		}
	case "reviews.changed":
		if event, ok := decode[struct {
			Item ReviewItem `json:"item"`
		}](data); ok && event.Item.ID != "" && event.Item.Version > 0 {
			s.upsertReview(event.Item)
		}
	case "tasks.changed":
		if event, ok := decode[struct {
			Task DurableTask `json:"task"`
		}](data); ok {
			s.AcceptDurableTask(event.Task)
		}
	case "snapshot":
		if snapshot, ok := decode[WireSnapshot](data); ok {
			s.apply(snapshot)
		}

	case "feedback.changed":
		if payload, ok := decode[struct {
			BotID string `json:"bot_id"`
		}](data); ok {
			s.emit(Event{Kind: EventFeedbackChanged, BotID: payload.BotID})
		}

	case "roster.changed":
		roster, ok := decode[WireRosterChanged](data)
		if !ok {
			return
		}
		s.Devices = s.Devices[:0:0]
		for _, device := range roster.Devices {
			s.Devices = append(s.Devices, ToDevice(device))
		}
		s.Bots = s.Bots[:0:0]
		for _, bot := range roster.Bots {
			s.Bots = append(s.Bots, ToBot(bot))
		}
		if roster.Routines != nil {
			s.Routines = s.Routines[:0:0]
			for _, routine := range roster.Routines {
				s.Routines = append(s.Routines, ToRoutine(routine))
			}
		}
		if roster.SharedLinks != nil {
			s.SharedLinks = roster.SharedLinks
		}
		if roster.AutoReview != nil {
			s.AutoReview = ToAutoReview(roster.AutoReview)
		}
		if roster.Providers != nil {
			s.Providers = ToProviders(roster.Providers)
		}
		if roster.Models != nil {
			s.Models = ToModels(roster.Models)
		}
		if roster.Playbooks != nil {
			s.Playbooks = *roster.Playbooks
		}
		var changed []string
		chats := make([]*Chat, 0, len(roster.Chats))
		for _, summary := range roster.Chats {
			existing := s.Chat(summary.ID)
			chat := ToChat(summary, existing)
			if existing == nil {
				chat.Messages = nil
			} else if !slices.Equal(existing.BotIDs, chat.BotIDs) || existing.CustomTitle != chat.CustomTitle {
				changed = append(changed, chat.ID)
			}
			chats = append(chats, chat)
		}
		s.Chats = chats
		s.sortChats()
		s.emit(Event{Kind: EventRosterChanged})
		s.emit(Event{Kind: EventChatsChanged})
		for _, id := range changed {
			s.emit(Event{Kind: EventChatChanged, ChatID: id})
		}

	case "message.added", "message.updated":
		payload, ok := decode[struct {
			ChatID  string      `json:"chat_id"`
			Message WireMessage `json:"message"`
		}](data)
		if !ok {
			return
		}
		if _, ok := s.retryNotes[payload.ChatID]; ok {
			delete(s.retryNotes, payload.ChatID)
			s.emit(Event{Kind: EventRespondingChanged, ChatID: payload.ChatID})
		}
		// The thinking bot's next message (a tool call, a reply) is where its thinking went.
		botID := str(payload.Message.Author.BotID)
		if name == "message.added" && botID != "" && s.thinkingBots[payload.ChatID] == botID {
			delete(s.thinkingBots, payload.ChatID)
		}
		s.upsert(ToMessage(payload.Message), payload.ChatID)

	case "message.removed":
		payload, ok := decode[struct {
			ChatID    string `json:"chat_id"`
			MessageID string `json:"message_id"`
		}](data)
		chat := s.Chat(payload.ChatID)
		if !ok || chat == nil {
			return
		}
		chat.Messages = slices.DeleteFunc(slices.Clone(chat.Messages), func(m *Message) bool { return m.ID == payload.MessageID })
		delete(s.commandStarts, payload.MessageID)
		s.emit(Event{Kind: EventMessageRemoved, ChatID: payload.ChatID, MessageID: payload.MessageID})
		s.noteOutput(nil, payload.MessageID, payload.ChatID)

	case "chat.removed":
		payload, ok := decode[struct {
			ChatID string `json:"chat_id"`
		}](data)
		if !ok {
			return
		}
		s.Chats = slices.DeleteFunc(slices.Clone(s.Chats), func(c *Chat) bool { return c.ID == payload.ChatID })
		s.runningJobs = slices.DeleteFunc(s.runningJobs, func(job runningJob) bool { return job.chatID == payload.ChatID })
		delete(s.outputMessages, payload.ChatID)
		delete(s.staleOutputs, payload.ChatID)
		s.emit(Event{Kind: EventChatsChanged})

	case "job.started":
		job, ok := decode[WireJobEvent](data)
		if !ok {
			return
		}
		// A snapshot may already list it.
		s.runningJobs = slices.DeleteFunc(s.runningJobs, func(running runningJob) bool {
			return running.id == "pending:"+job.ChatID || running.id == job.JobID
		})
		s.runningJobs = append(s.runningJobs, runningJob{id: job.JobID, chatID: job.ChatID, botID: job.BotID, routineID: str(job.RoutineID)})
		if _, ok := s.jobStarts[job.JobID]; !ok {
			s.jobStarts[job.JobID] = time.Now()
		}
		s.emit(Event{Kind: EventRespondingChanged, ChatID: job.ChatID})
		s.emit(Event{Kind: EventChatsChanged})

	case "job.finished":
		job, ok := decode[WireJobEvent](data)
		if !ok {
			return
		}
		s.runningJobs = slices.DeleteFunc(s.runningJobs, func(running runningJob) bool { return running.id == job.JobID })
		delete(s.retryNotes, job.ChatID)
		if job.BotID == "" || s.thinkingBots[job.ChatID] == job.BotID {
			delete(s.thinkingBots, job.ChatID)
		}
		s.emit(Event{Kind: EventRespondingChanged, ChatID: job.ChatID})
		s.emit(Event{Kind: EventChatsChanged})
		startedAt, seen := s.jobStarts[job.JobID]
		delete(s.jobStarts, job.JobID)
		if seen && job.BotID != "" {
			s.emit(Event{Kind: EventTurnFinished, ChatID: job.ChatID, BotID: job.BotID, StartedAt: startedAt})
		}

	case "job.retry":
		retry, ok := decode[WireJobRetry](data)
		if !ok {
			return
		}
		seconds := max(1, (retry.DelayMS+500)/1000)
		s.retryNotes[retry.ChatID] = L("Retrying (%d of %d) in %d s", retry.Attempt, retry.MaxAttempts, seconds)
		s.emit(Event{Kind: EventRespondingChanged, ChatID: retry.ChatID})

	case "job.thinking":
		job, ok := decode[struct {
			ChatID string `json:"chat_id"`
			BotID  string `json:"bot_id"`
		}](data)
		if !ok {
			return
		}
		s.thinkingBots[job.ChatID] = job.BotID
		s.emit(Event{Kind: EventRespondingChanged, ChatID: job.ChatID})

	case "chat.usage":
		payload, ok := decode[struct {
			ChatID string        `json:"chat_id"`
			Usage  WireChatUsage `json:"usage"`
		}](data)
		chat := s.Chat(payload.ChatID)
		if !ok || chat == nil {
			return
		}
		chat.Usage = ToUsage(payload.Usage)
		s.emit(Event{Kind: EventChatChanged, ChatID: payload.ChatID})
	case "budgets.changed":
		if payload, ok := decode[struct {
			Budgets []BudgetState `json:"budgets"`
		}](data); ok {
			s.Budgets = slices.Clone(payload.Budgets)
			s.emit(Event{Kind: EventBudgetsChanged})
		}

	case "relay.status":
		status, ok := decode[WireRelayStatus](data)
		if !ok {
			return
		}
		s.RelayConnected = status.Connected
		s.RelayUpdateRequired = flag(status.UpdateRequired)
		s.RelayError = ""
		if status.Error != nil {
			s.RelayError = status.Error.Message
		}
		if status.URL != nil {
			s.RelayURL = *status.URL
		}
		s.emit(Event{Kind: EventRosterChanged})

	case "identity.changed":
		payload, ok := decode[struct {
			HasIdentity bool `json:"has_identity"`
		}](data)
		if !ok {
			return
		}
		s.HasIdentity = &payload.HasIdentity
		if !payload.HasIdentity {
			s.applyAttention(DefaultAttention())
		}
		s.emit(Event{Kind: EventIdentityChanged})
	}
}

func (s *Store) upsert(message *Message, chatID string) {
	chat := s.Chat(chatID)
	if chat == nil {
		return
	}
	s.noteCommand(message, chatID)
	s.noteOutput(message, "", chatID)
	if index := slices.IndexFunc(chat.Messages, func(m *Message) bool { return m.ID == message.ID }); index >= 0 {
		messages := slices.Clone(chat.Messages)
		messages[index] = message
		chat.Messages = messages
		s.emit(Event{Kind: EventMessageChanged, ChatID: chatID, MessageID: message.ID})
		if message.State.Kind == StateComplete {
			s.RefreshChatList()
		}
		return
	}
	chat.Messages = append(slices.Clone(chat.Messages), message)
	s.emit(Event{Kind: EventMessageAdded, ChatID: chatID, MessageID: message.ID})
	s.sortChats()
	s.emit(Event{Kind: EventChatsChanged})
}

// MARK: - Observation

// Subscribe calls listener, on the main thread, with every event from now on.
func (s *Store) Subscribe(listener func(Event)) {
	s.listeners = append(s.listeners, listener)
}

func (s *Store) emit(event Event) {
	if s.isApplyingBootstrap {
		return
	}
	for _, listener := range slices.Clone(s.listeners) {
		listener(event)
	}
}

// MARK: - Lookup

func findChat(chats []*Chat, id string) *Chat {
	for _, chat := range chats {
		if chat.ID == id {
			return chat
		}
	}
	return nil
}

func (s *Store) Bot(id string) *Bot {
	for _, bot := range s.Bots {
		if bot.ID == id {
			return bot
		}
	}
	return nil
}

func (s *Store) Device(id string) *Device {
	for _, device := range s.Devices {
		if device.ID == id {
			return device
		}
	}
	return nil
}

// Runners are the Devices that can be assigned bots: those with a desktop OS.
func (s *Store) Runners() []*Device {
	var out []*Device
	for _, device := range s.Devices {
		if device.IsRunner() {
			out = append(out, device)
		}
	}
	return out
}

func (s *Store) Chat(id string) *Chat { return findChat(s.Chats, id) }

func (s *Store) BotsIn(chat *Chat) []*Bot {
	var out []*Bot
	for _, id := range chat.BotIDs {
		if bot := s.Bot(id); bot != nil {
			out = append(out, bot)
		}
	}
	return out
}

func (s *Store) BotsOn(runnerID string) []*Bot {
	var out []*Bot
	for _, bot := range s.Bots {
		if bot.RunnerID == runnerID {
			out = append(out, bot)
		}
	}
	return out
}

func (s *Store) Routine(id string) *Routine {
	for _, routine := range s.Routines {
		if routine.ID == id {
			return routine
		}
	}
	return nil
}

// RoutinesFor is a bot's routines, oldest first, with the running state from the turns in flight.
func (s *Store) RoutinesFor(botID string) []Routine {
	var out []Routine
	for _, routine := range s.Routines {
		if routine.BotID != botID {
			continue
		}
		copy := *routine
		if !copy.IsRunning {
			copy.IsRunning = slices.ContainsFunc(s.runningJobs, func(job runningJob) bool { return job.routineID == routine.ID })
		}
		out = append(out, copy)
	}
	slices.SortStableFunc(out, func(a, b Routine) int { return a.CreatedAt.Compare(b.CreatedAt) })
	return out
}

func (s *Store) ThisDevice() *Device {
	for _, device := range s.Devices {
		if device.IsThisDevice {
			return device
		}
	}
	return nil
}

func (s *Store) Title(chat *Chat) string {
	if chat.IsGroup() && chat.CustomTitle != "" {
		return chat.CustomTitle
	}
	var names []string
	for _, id := range chat.BotIDs {
		if bot := s.Bot(id); bot != nil {
			names = append(names, bot.Name)
		}
	}
	if len(names) == 0 {
		return L("New Chat")
	}
	return strings.Join(names, ", ")
}

func (s *Store) Subtitle(chat *Chat) string {
	members := s.BotsIn(chat)
	if chat.IsDM() && len(members) > 0 {
		only := members[0]
		host := L("unassigned")
		if device := s.Device(only.RunnerID); device != nil {
			host = device.Name
		}
		return L("%@ on %@", ProviderName(only.Provider, s.Providers), host)
	}
	hosts := map[string]bool{}
	var first string
	for _, bot := range members {
		if device := s.Device(bot.RunnerID); device != nil {
			if !hosts[device.Name] && first == "" {
				first = device.Name
			}
			hosts[device.Name] = true
		}
	}
	runnerLabel := L("%d Runners", len(hosts))
	if len(hosts) == 1 {
		runnerLabel = first
	}
	botLabel := L("%d bots", len(members))
	if len(members) == 1 {
		botLabel = L("1 bot")
	}
	return L("Group · %@ · %@", botLabel, runnerLabel)
}

func (s *Store) botName(id, fallback string) string {
	if bot := s.Bot(id); bot != nil {
		return bot.Name
	}
	return fallback
}

// Preview is the chat list's line under a chat's title: the last thing worth previewing.
func (s *Store) Preview(chat *Chat) string {
	// Tool calls never are, except a sent message.
	var last *Message
	for i := len(chat.Messages) - 1; i >= 0; i-- {
		message := chat.Messages[i]
		if message.Body.Kind != BodyTool || message.Body.Tool.IsSentMessage() {
			last = message
			break
		}
	}
	if last == nil {
		return L("No messages yet")
	}
	var body string
	content := last.Body
	switch content.Kind {
	case BodyText:
		body = content.Text
		if body == "" {
			body = AttachmentSummary(last.Attachments)
		}
	case BodyTool:
		target := L("a teammate")
		if content.Tool.TargetBotID != "" {
			target = s.botName(content.Tool.TargetBotID, target)
		}
		body = L("Messaged %@: %@", target, content.Tool.Detail)
	case BodyHandoff:
		if slices.Contains(chat.BotIDs, content.Handoff.To) && !slices.Contains(chat.BotIDs, content.Handoff.From) {
			body = L("Message from %@: %@", s.botName(content.Handoff.From, L("a teammate")), content.Handoff.Reason)
		} else {
			body = L("Handed off to %@", s.botName(content.Handoff.To, L("a teammate")))
		}
	case BodyNotice:
		body = content.Text
	case BodyPermission:
		who := s.botName(last.Author.BotID, L("A bot"))
		body = who + " " + content.Request.VerbPhrase()
	}
	flattened := strings.TrimSpace(strings.NewReplacer("\n", " ", "**", "", "`", "").Replace(body))
	if chat.IsGroup() && last.Author.Kind == AuthorBot && content.Kind == BodyText {
		return s.botName(last.Author.BotID, L("Bot")) + ": " + flattened
	}
	return flattened
}

// MARK: - Connection

func (s *Store) SetConnected(connected bool) {
	if s.IsConnected == connected {
		return
	}
	s.IsConnected = connected
	s.emit(Event{Kind: EventConnectionChanged})
}

// MARK: - Requests

// perform is a fire-and-forget request. A failure is logged and the store re-syncs from the CLI,
// so an optimistic change that the CLI rejected gets rolled back.
func (s *Store) perform(method string, params any) {
	if s.IsMock {
		return
	}
	go func() {
		err := s.request(method, params, nil)
		if err == nil {
			return
		}
		log.Printf("%s failed: %s", method, ErrorText(err))
		snapshot, err := call[WireSnapshot](s, "bootstrap", nil)
		if err == nil {
			s.post(func() { s.apply(snapshot) })
		}
	}()
}

// MARK: - Chat mutation

func (s *Store) sortChats() {
	chats := slices.Clone(s.Chats)
	slices.SortStableFunc(chats, func(lhs, rhs *Chat) int {
		if lhs.IsPinned != rhs.IsPinned {
			if lhs.IsPinned {
				return -1
			}
			return 1
		}
		a, b := lhs.LastActivity(), rhs.LastActivity()
		if a.Equal(b) {
			return strings.Compare(lhs.ID, rhs.ID)
		}
		return b.Compare(a)
	})
	s.Chats = chats
}

// DM is the one DM with this bot, created on first use. DMs are keyed by the bot, so opening one
// twice lands in the same thread.
func (s *Store) DM(botID string) string {
	for _, chat := range s.Chats {
		if chat.IsDM() && len(chat.BotIDs) == 1 && chat.BotIDs[0] == botID {
			return chat.ID
		}
	}
	return s.CreateChat(ChatDM, []string{botID}, "")
}

func (s *Store) CreateChat(kind ChatKind, botIDs []string, title string) string {
	members := botIDs
	if kind == ChatDM {
		members = botIDs[:min(1, len(botIDs))]
	} else {
		members = botIDs[:min(MaxGroupBots, len(botIDs))]
	}
	chat := &Chat{ID: ShortID("chat"), Kind: kind, BotIDs: slices.Clone(members), CreatedAt: time.Now()}
	if kind == ChatGroup {
		chat.CustomTitle = title
	}
	s.Chats = append([]*Chat{chat}, s.Chats...)
	s.sortChats()
	s.emit(Event{Kind: EventChatsChanged})
	s.perform("chats.create", map[string]any{"id": chat.ID, "kind": string(kind), "bot_ids": members, "title": title})
	return chat.ID
}

// NewBot is what CreateBot makes a bot from.
type NewBot struct {
	Name        string
	Description string
	SymbolName  string
	Accent      Accent
	RunnerID    string
	Provider    ProviderKind
	Model       string
	Thinking    string
	TemplateID  string
	Greeting    string
}

func (s *Store) CreateBot(options NewBot) string {
	bot := &Bot{
		ID:          ShortID("bot"),
		Name:        options.Name,
		Description: options.Description,
		SymbolName:  options.SymbolName,
		Accent:      options.Accent,
		RunnerID:    options.RunnerID,
		Provider:    options.Provider,
		Model:       options.Model,
		Thinking:    options.Thinking,
		CreatedAt:   time.Now(),
	}
	s.Bots = append(slices.Clone(s.Bots), bot)
	s.emit(Event{Kind: EventRosterChanged})
	s.emit(Event{Kind: EventChatsChanged})

	// The CLI gives every bot its direct chat; create it under the id the app will open.
	if !s.IsMock {
		chat := &Chat{ID: ShortID("chat"), Kind: ChatDM, BotIDs: []string{bot.ID}, CreatedAt: time.Now()}
		s.Chats = append([]*Chat{chat}, s.Chats...)
		s.sortChats()
		s.emit(Event{Kind: EventChatsChanged})
		params := map[string]any{
			"id":          bot.ID,
			"name":        options.Name,
			"description": options.Description,
			"symbol_name": options.SymbolName,
			"accent":      string(options.Accent),
			"runner_id":   options.RunnerID,
			"provider":    options.Provider,
			"model":       options.Model,
			"thinking":    options.Thinking,
			"chat_id":     chat.ID,
		}
		if options.TemplateID != "" {
			params["template_id"] = options.TemplateID
			params["greeting"] = options.Greeting
		}
		s.perform("bots.create", params)
	}
	return bot.ID
}

// AddBotFromTemplate adds a bot from a marketplace template on `runnerID` and answers with its
// direct chat. The CLI gives it the template's routines, paused, and a first turn that answers the
// user's greeting.
func (s *Store) AddBotFromTemplate(template BotTemplate, runnerID string) string {
	botID := s.CreateBot(NewBot{
		Name:        template.Name,
		Description: template.Description,
		SymbolName:  template.SymbolName,
		Accent:      template.Accent,
		RunnerID:    runnerID,
		Provider:    s.PreferredProvider(),
		TemplateID:  template.ID,
		Greeting:    L("Hi %@, introduce yourself.", template.Name),
	})
	return s.DM(botID)
}

// PreferredProvider is the provider a bot made without asking runs with: the first one the
// account connected that a bot can run with.
func (s *Store) PreferredProvider() ProviderKind {
	for _, kind := range s.ProviderKinds() {
		if credential := s.Credential(kind); credential != nil && credential.IsConnected {
			return kind
		}
	}
	return "deepseek"
}

// ProviderKinds is every provider a bot can run with: the built-in ones, then the ones the user
// added, except those of decision models.
func (s *Store) ProviderKinds() []ProviderKind {
	out := slices.Clone(ProviderKinds)
	for _, provider := range s.Providers {
		if IsCustomKind(provider.Kind) && !provider.Decides() {
			out = append(out, provider.Kind)
		}
	}
	return out
}

// ReviewProviderKinds are the providers Auto-review can run a model of: every one the account has
// connected, in the order the CLI lists them.
func (s *Store) ReviewProviderKinds() []ProviderKind {
	var out []ProviderKind
	for _, provider := range s.Providers {
		if provider.IsConnected {
			out = append(out, provider.Kind)
		}
	}
	return out
}

func (s *Store) rosterTouched(botID string) {
	s.emit(Event{Kind: EventRosterChanged})
	s.emit(Event{Kind: EventChatsChanged})
	if botID != "" {
		for _, chat := range s.Chats {
			if slices.Contains(chat.BotIDs, botID) {
				s.emit(Event{Kind: EventChatChanged, ChatID: chat.ID})
			}
		}
	}
}

// UpdateBotProfile renames a bot, and changes its description when `description` is not nil.
func (s *Store) UpdateBotProfile(id, name string, description *string) {
	bot := s.Bot(id)
	if bot == nil {
		return
	}
	bot.Name = name
	params := map[string]any{"id": id, "name": name}
	if description != nil {
		bot.Description = *description
		params["description"] = *description
	}
	s.rosterTouched("")
	s.perform("bots.update", params)
}

// SetBotLook is the bot's symbol and accent, the look under and behind its image.
func (s *Store) SetBotLook(id, symbolName string, accent Accent) {
	bot := s.Bot(id)
	if bot == nil {
		return
	}
	bot.SymbolName, bot.Accent = symbolName, accent
	s.rosterTouched("")
	s.perform("bots.update", map[string]any{"id": id, "symbol_name": symbolName, "accent": string(accent)})
}

// AvatarFile is a picture on this computer for SetBotAvatar: a prepared square PNG.
type AvatarFile struct {
	Path string
	Name string
	Mime string
	Size int64
}

// SetBotAvatar sets a custom profile image from a file on this computer (nil removes the current
// one). The CLI copies it into its store, uploads it as a `file` blob, and names it in the roster,
// which comes back as the bot's avatar for every Device.
func (s *Store) SetBotAvatar(id string, file *AvatarFile) {
	bot := s.Bot(id)
	if bot == nil {
		return
	}
	if file == nil {
		bot.Avatar = nil
		s.rosterTouched("")
		s.perform("bots.update", map[string]any{"id": id, "avatar": nil})
		return
	}
	mime := file.Mime
	if !strings.HasPrefix(mime, "image/") {
		mime = "image/png"
	}
	attachment := Attachment{ID: NewAttachmentID(), Name: file.Name, Mime: mime, Size: file.Size}
	s.attachmentFiles[attachment.ID] = file.Path
	bot.Avatar = &attachment
	s.rosterTouched("")
	s.perform("bots.update", map[string]any{"id": id, "avatar": map[string]any{"id": attachment.ID, "path": file.Path, "name": attachment.Name, "mime": attachment.Mime}})
}

// SetBotRuntime is the provider, model, and thinking level a bot runs with. Empty means the
// provider's default.
func (s *Store) SetBotRuntime(id string, provider ProviderKind, model, thinking string) {
	bot := s.Bot(id)
	if bot == nil {
		return
	}
	bot.Provider, bot.Model, bot.Thinking = provider, model, thinking
	s.rosterTouched(id)
	s.perform("bots.update", map[string]any{"id": id, "provider": provider, "model": model, "thinking": thinking})
}

// CompactChat summarizes the chat's older part for its bot now, on the Runner.
func (s *Store) CompactChat(id string) {
	s.perform("chats.compact", map[string]any{"chat_id": id})
}

// MARK: - Plugins

// Marketplace answers the marketplace: its plugins, each with the Runners that have it, and its bots.
func (s *Store) Marketplace(done func(Marketplace, error)) {
	if s.IsMock {
		if s.mockMarketplace == nil {
			market := mockMarketplace()
			market.Packs = demoWorkflowPacks()
			s.mockMarketplace = &market
		}
		market := *s.mockMarketplace
		s.post(func() { done(market, nil) })
		return
	}
	Async(s, func() (Marketplace, error) {
		wire, err := call[WireMarketplace](s, "marketplace", nil)
		return ToMarketplace(wire), err
	}, done)
}

type pluginReply struct {
	Status WirePluginStatus `json:"status"`
}

func readyPlugin(id string) InstalledPlugin {
	return InstalledPlugin{ID: id, Name: id, State: PluginReady, Detail: "Ready"}
}

// InstallPlugin installs a marketplace plugin on a Runner (here, or sealed to that Runner).
func (s *Store) InstallPlugin(pluginID, runnerID string, done func(InstalledPlugin, error)) {
	if s.IsMock {
		s.post(func() { done(readyPlugin(pluginID), nil) })
		return
	}
	Async(s, func() (InstalledPlugin, error) {
		reply, err := call[pluginReply](s, "plugins.install", map[string]any{"runner_id": runnerID, "plugin_id": pluginID})
		return ToPlugin(reply.Status), err
	}, func(plugin InstalledPlugin, err error) {
		if err == nil {
			s.rememberPlugin(runnerID, plugin)
		}
		done(plugin, err)
	})
}

func (s *Store) UninstallPlugin(pluginID, runnerID string, done func(error)) {
	s.simple(func(err error) {
		if err == nil {
			if runner := s.Device(runnerID); runner != nil {
				runner.Plugins = slices.DeleteFunc(runner.Plugins, func(p InstalledPlugin) bool { return p.ID == pluginID })
				s.emit(Event{Kind: EventRosterChanged})
			}
		}
		if done != nil {
			done(err)
		}
	}, "plugins.uninstall", map[string]any{"runner_id": runnerID, "plugin_id": pluginID})
}

// simple is a request whose answer is only whether it went through.
func (s *Store) simple(done func(error), method string, params any) {
	if s.IsMock {
		if done != nil {
			s.post(func() { done(nil) })
		}
		return
	}
	Async(s, func() (struct{}, error) {
		return struct{}{}, s.request(method, params, nil)
	}, func(_ struct{}, err error) {
		if done != nil {
			done(err)
		}
	})
}

func (s *Store) PluginDetail(pluginID, runnerID string, done func(PluginDetail, error)) {
	if s.IsMock {
		status := readyPlugin(pluginID)
		if device := s.Device(runnerID); device != nil {
			for _, plugin := range device.Plugins {
				if plugin.ID == pluginID {
					status = plugin
				}
			}
		}
		detail := PluginDetail{
			Status:    status,
			Variables: []PluginDetailVariable{{Name: "GITHUB_TOKEN", Description: "A personal access token, instead of signing in.", Secret: true}},
			Servers:   []PluginDetailServer{{Name: "github", Kind: "http", URL: "https://api.githubcopilot.com/mcp/", OAuth: true, SignedIn: status.State == PluginReady}},
		}
		if status.ServiceID != "" {
			detail.Variables, detail.Servers = nil, nil
			for _, manifest := range mockMarketplace().Plugins {
				if manifest.ID != status.ServiceID {
					continue
				}
				detail.Homepage, detail.Skills = manifest.Homepage, manifest.Skills
				for _, variable := range manifest.Variables {
					field := PluginDetailVariable{Name: variable.Name, Description: variable.Description, Secret: variable.Secret, Required: variable.Required}
					if !variable.Secret && status.State != PluginNeedsSetup {
						field.IsSet, field.Value = true, "demo-client-id"
					}
					detail.Variables = append(detail.Variables, field)
				}
				for _, server := range manifest.Servers {
					detail.Servers = append(detail.Servers, PluginDetailServer{Name: server.Name, Kind: "http", URL: server.Address, OAuth: server.SignsIn, SignedIn: status.State == PluginReady})
				}
			}
		}
		if pluginID == BrowserPluginID {
			// Browser runs on the Runner and signs in to nothing itself.
			detail = PluginDetail{Status: status, Homepage: "https://github.com/microsoft/playwright-mcp", Skills: []NamedText{{Name: "Reading a page", Description: "How to read a page without filling the context."}}}
		}
		s.post(func() { done(detail, nil) })
		return
	}
	Async(s, func() (PluginDetail, error) {
		wire, err := call[WirePluginDetail](s, "plugins.detail", map[string]any{"runner_id": runnerID, "plugin_id": pluginID})
		return ToPluginDetail(wire), err
	}, done)
}

// SetPluginVariables sets variables on the Runner; a secret goes out in the request and is never
// read back.
func (s *Store) SetPluginVariables(pluginID, runnerID string, variables map[string]string, done func(InstalledPlugin, error)) {
	if s.IsMock {
		if runner := s.Device(runnerID); runner != nil {
			for _, plugin := range runner.Plugins {
				if plugin.ID == pluginID && plugin.ServiceID != "" {
					s.post(func() { done(plugin, nil) })
					return
				}
			}
		}
		s.post(func() { done(readyPlugin(pluginID), nil) })
		return
	}
	Async(s, func() (InstalledPlugin, error) {
		reply, err := call[pluginReply](s, "plugins.set_variables", map[string]any{"runner_id": runnerID, "plugin_id": pluginID, "variables": variables})
		return ToPlugin(reply.Status), err
	}, done)
}

// ConnectPlugin starts a plugin's sign-in for the Runner; the browser opens on this computer.
func (s *Store) ConnectPlugin(pluginID, runnerID string, done func(error)) {
	if s.mockIntegrationState(pluginID, runnerID, PluginReady, "Connected", done) {
		return
	}
	s.simple(done, "plugins.connect", map[string]any{"runner_id": runnerID, "plugin_id": pluginID})
}

// SignOutPlugin forgets a plugin server's sign-in on its Runner. Nothing is revoked at the server;
// the plugin's next use asks for a sign-in again.
func (s *Store) SignOutPlugin(pluginID, runnerID, server string, done func(error)) {
	if s.mockIntegrationState(pluginID, runnerID, PluginNeedsAuth, "Sign in", done) {
		return
	}
	s.simple(done, "plugins.sign_out", map[string]any{"runner_id": runnerID, "plugin_id": pluginID, "server": server})
}

// MARK: - MCP servers

func (s *Store) mockMcpServers(runnerID string) []McpServer {
	servers, ok := s.mockMcp[runnerID]
	if !ok {
		if runnerID == "dev-workbench" {
			servers = mockMcpServers()
		}
		s.mockMcp[runnerID] = servers
	}
	return servers
}

// setMockMcpServers is the demo's Runner taking its servers as the CLI would: the ones that run
// are its plugins.
func (s *Store) setMockMcpServers(runnerID string, servers []McpServer) {
	s.mockMcp[runnerID] = servers
	device := s.Device(runnerID)
	if device == nil {
		return
	}
	var plugins []InstalledPlugin
	for _, plugin := range device.Plugins {
		if plugin.Source != "mcp.json" {
			plugins = append(plugins, plugin)
		}
	}
	for _, server := range servers {
		if server.Enabled && server.Status != nil {
			plugins = append(plugins, *server.Status)
		}
	}
	device.Plugins = plugins
	s.emit(Event{Kind: EventRosterChanged})
}

func (s *Store) mockMcpServer(runnerID, name string) (McpServer, int, error) {
	servers := s.mockMcpServers(runnerID)
	for i, server := range servers {
		if server.Name == name {
			return server, i, nil
		}
	}
	return McpServer{}, -1, &RequestError{L("No server named %@ in mcp.json.", name)}
}

type mcpServerReply struct {
	Server WireMcpServer `json:"server"`
}

func (s *Store) mcpServerCall(method string, params map[string]any, done func(McpServer, error)) {
	Async(s, func() (McpServer, error) {
		reply, err := call[mcpServerReply](s, method, params)
		return ToMcpServer(reply.Server), err
	}, done)
}

// McpServers answers a Runner's mcp.json: every server in it, usable or not, here or sealed to
// that Runner.
func (s *Store) McpServers(runnerID string, done func(McpFile, error)) {
	if s.IsMock {
		file := McpFile{Path: "~/.lorca/mcp.json", Servers: s.mockMcpServers(runnerID)}
		s.post(func() { done(file, nil) })
		return
	}
	Async(s, func() (McpFile, error) {
		wire, err := call[WireMcpFile](s, "mcp.list", map[string]any{"runner_id": runnerID})
		return ToMcpFile(wire), err
	}, done)
}

// McpServer answers one server with the tools it offered when it last connected.
func (s *Store) McpServer(name, runnerID string, done func(McpServer, error)) {
	if s.IsMock {
		server, _, err := s.mockMcpServer(runnerID, name)
		s.post(func() { done(server, err) })
		return
	}
	s.mcpServerCall("mcp.get", map[string]any{"runner_id": runnerID, "name": name}, done)
}

// SaveMcpServer adds a server, or saves the one `previousName` names under `name`. The CLI checks
// the entry and writes it to the Runner's mcp.json, where the server starts once to list its tools.
func (s *Store) SaveMcpServer(runnerID, name string, entry McpEntry, previousName string, done func(McpServer, error)) {
	if s.IsMock {
		servers := s.mockMcpServers(runnerID)
		if (previousName == "" || previousName != name) && slices.ContainsFunc(servers, func(each McpServer) bool { return each.Name == name }) {
			err := &RequestError{L("mcp.json already has a server named %@.", name)}
			s.post(func() { done(McpServer{}, err) })
			return
		}
		id := strings.Trim(nonAlnum.ReplaceAllString(strings.ToLower(name), "-"), "-")
		if id == "" {
			id = "server"
		}
		icon := "terminal"
		if entry.URL() != "" {
			icon = "globe"
		}
		enabled := !entry.Disabled()
		server := McpServer{Name: name, ID: id, Enabled: enabled, Entry: entry, ToolCount: 2, Tools: []McpTool{
			{Name: "echo", Description: "Echo back what you send.", ReadOnly: true},
			{Name: "write_note", Description: "Write a note."},
		}}
		if enabled {
			server.Status = &InstalledPlugin{ID: id, Name: name, Description: entry.Description(), Icon: icon, State: PluginReady, Detail: "Ready", Source: "mcp.json"}
		}
		key := name
		if previousName != "" {
			key = previousName
		}
		next := slices.Clone(servers)
		if index := slices.IndexFunc(next, func(each McpServer) bool { return each.Name == key }); index >= 0 {
			next[index] = server
		} else {
			next = append(next, server)
		}
		s.setMockMcpServers(runnerID, next)
		s.post(func() { done(server, nil) })
		return
	}
	params := map[string]any{"runner_id": runnerID, "name": name, "config": map[string]any(entry)}
	if previousName != "" {
		params["previous_name"] = previousName
	}
	s.mcpServerCall("mcp.save", params, done)
}

// RemoveMcpServer removes a server from the Runner's mcp.json, with its sign-in.
func (s *Store) RemoveMcpServer(runnerID, name string, done func(error)) {
	if s.IsMock {
		s.setMockMcpServers(runnerID, slices.DeleteFunc(slices.Clone(s.mockMcpServers(runnerID)), func(each McpServer) bool { return each.Name == name }))
		s.post(func() { done(nil) })
		return
	}
	s.simple(done, "mcp.remove", map[string]any{"runner_id": runnerID, "name": name})
}

// SetMcpServerEnabled turns a server on or off; off, no bot sees it and it never starts.
func (s *Store) SetMcpServerEnabled(runnerID, name string, enabled bool, done func(McpServer, error)) {
	if s.IsMock {
		server, index, err := s.mockMcpServer(runnerID, name)
		if err == nil {
			entry := server.Entry.Clone()
			if enabled {
				delete(entry, "disabled")
			} else {
				entry["disabled"] = true
			}
			server.Enabled, server.Entry = enabled, entry
			if enabled {
				if server.Status == nil {
					icon := "terminal"
					if entry.URL() != "" {
						icon = "globe"
					}
					server.Status = &InstalledPlugin{ID: server.ID, Name: name, Description: entry.Description(), Icon: icon, State: PluginReady, Detail: "Ready", Source: "mcp.json"}
				}
			} else {
				server.Status = nil
			}
			next := slices.Clone(s.mockMcpServers(runnerID))
			next[index] = server
			s.setMockMcpServers(runnerID, next)
		}
		s.post(func() { done(server, err) })
		return
	}
	s.mcpServerCall("mcp.set_enabled", map[string]any{"runner_id": runnerID, "name": name, "enabled": enabled}, done)
}

// ReconnectMcpServer connects a server and waits for it: from scratch (`fresh`), or taking a
// connection it has or one under way. Its state says how it went.
func (s *Store) ReconnectMcpServer(runnerID, name string, fresh bool, done func(McpServer, error)) {
	if s.IsMock {
		s.later(900*time.Millisecond, func() { s.McpServer(name, runnerID, done) })
		return
	}
	s.mcpServerCall("mcp.reconnect", map[string]any{"runner_id": runnerID, "name": name, "fresh": fresh}, done)
}

// ReloadMcpServers reads the Runner's mcp.json again, after an edit made outside Lorca.
func (s *Store) ReloadMcpServers(runnerID string, done func(McpFile, error)) {
	if s.IsMock {
		s.McpServers(runnerID, done)
		return
	}
	Async(s, func() (McpFile, error) {
		wire, err := call[WireMcpFile](s, "mcp.reload", map[string]any{"runner_id": runnerID})
		return ToMcpFile(wire), err
	}, done)
}

// SetMcpToolHidden offers one of a server's tools to bots, or keeps it from them, in the Runner's
// mcp.json. The server keeps its connection.
func (s *Store) SetMcpToolHidden(runnerID, name, tool string, hidden bool, done func(McpServer, error)) {
	if s.IsMock {
		server, index, err := s.mockMcpServer(runnerID, name)
		if err == nil {
			tools := slices.Clone(server.Tools)
			for i := range tools {
				if tools[i].Name == tool {
					tools[i].Hidden = hidden
				}
			}
			server.Tools = tools
			next := slices.Clone(s.mockMcpServers(runnerID))
			next[index] = server
			s.setMockMcpServers(runnerID, next)
		}
		s.post(func() { done(server, err) })
		return
	}
	s.mcpServerCall("mcp.hide_tool", map[string]any{"runner_id": runnerID, "name": name, "tool": tool, "hidden": hidden}, done)
}

// SignOutMcpServer forgets a remote server's sign-in on its Runner; its next use asks for one again.
func (s *Store) SignOutMcpServer(runnerID, name string, done func(McpServer, error)) {
	if s.IsMock {
		server, index, err := s.mockMcpServer(runnerID, name)
		if err == nil {
			server.SignedIn = false
			if server.Status != nil {
				status := *server.Status
				status.State, status.Detail = PluginNeedsAuth, "Sign in"
				server.Status = &status
			}
			next := slices.Clone(s.mockMcpServers(runnerID))
			next[index] = server
			s.setMockMcpServers(runnerID, next)
		}
		s.post(func() { done(server, err) })
		return
	}
	s.mcpServerCall("mcp.sign_out", map[string]any{"runner_id": runnerID, "name": name}, done)
}

// ParseMcpJSON answers the servers pasted JSON holds, in any app's spelling; the CLI on this
// computer reads it.
func (s *Store) ParseMcpJSON(text string, done func([]ParsedServer, error)) {
	if s.IsMock {
		servers, err := ParseLocally(text)
		s.post(func() { done(servers, err) })
		return
	}
	Async(s, func() ([]ParsedServer, error) {
		wire, err := call[WireParsedServers](s, "mcp.parse", map[string]any{"text": text})
		return ToParsedServers(wire), err
	}, done)
}

// SetAutoReview replaces Auto-review (the switch and the rules); the change shows at once and the
// CLI's roster event confirms it. A new rule gets its id from the CLI. The model that reviews is
// left out, so it stays as picked.
func (s *Store) SetAutoReview(value AutoReview) {
	s.AutoReview = value
	s.emit(Event{Kind: EventRosterChanged})
	rules := make([]map[string]any, 0, len(value.Rules))
	for _, rule := range value.Rules {
		entry := map[string]any{"id": rule.ID, "text": rule.Text, "behavior": rule.Behavior}
		if rule.Tool != "" {
			entry["tool"] = rule.Tool
		}
		rules = append(rules, entry)
	}
	s.perform("auto_review.set", map[string]any{"is_enabled": value.IsEnabled, "rules": rules})
}

// SetReviewProvider picks the provider Auto-review runs the review model of, or "" for the bot's
// own.
func (s *Store) SetReviewProvider(provider ProviderKind) {
	s.AutoReview.Provider = provider
	s.emit(Event{Kind: EventRosterChanged})
	params := map[string]any{"provider": nil}
	if provider != "" {
		params["provider"] = provider
	}
	s.perform("auto_review.set", params)
}

// ReviewModel is the review model picked for a provider; empty for its default.
func (s *Store) ReviewModel(kind ProviderKind) string { return s.AutoReview.Models[kind] }

// SetReviewModel picks a provider's review model, or "" to put back its default. The other
// providers' stay as they are.
func (s *Store) SetReviewModel(model string, kind ProviderKind) {
	models := maps.Clone(s.AutoReview.Models)
	if models == nil {
		models = map[ProviderKind]string{}
	}
	var value any
	if model == "" {
		delete(models, kind)
	} else {
		models[kind], value = model, model
	}
	s.AutoReview.Models = models
	s.emit(Event{Kind: EventRosterChanged})
	s.perform("auto_review.set", map[string]any{"models": map[string]any{kind: value}})
}

// AnswerPermission answers a question: a permission card's, or a command card's. `allow`,
// `always`, or `deny`. The CLI confirms with the card's new state.
func (s *Store) AnswerPermission(chatID, messageID, decision string) {
	s.Update(messageID, chatID, func(message *Message) {
		switch body := message.Body; {
		case body.Kind == BodyPermission:
			request := *body.Request
			switch decision {
			case "always":
				request.Decision = DecisionAlways
			case "deny":
				// An access request is only ever dismissed.
				request.Decision = DecisionDenied
				if request.IsAccess() {
					request.Decision = DecisionDismissed
				}
			default:
				request.Decision = DecisionAllowed
			}
			if request.Tool == "connect" && request.Decision == DecisionAllowed {
				request.Summary = L("Starting the sign-in…")
			}
			message.Body.Request = &request
		case body.Kind == BodyTool && body.Tool.Run != nil && body.Tool.Run.State == CommandAsking:
			tool := *body.Tool
			run := *tool.Run
			run.State = CommandRunning
			if decision == "deny" {
				run.State = CommandDenied
			}
			tool.Run = &run
			message.Body.Tool = &tool
		}
	})
	s.perform("chats.permission", map[string]any{"chat_id": chatID, "message_id": messageID, "decision": decision})
}

// MARK: - Commands

// AnswerCommand types the user's answer into a command running in its terminal, then Return. The
// CLI writes it to the terminal, or seals it to the bot's Runner, and keeps nothing. Fails with why
// it could not, such as that Runner being offline.
func (s *Store) AnswerCommand(chatID, messageID, text string, done func(error)) {
	if s.IsMock {
		s.finishMockCommand(chatID, messageID, CommandExited)
		s.post(func() { done(nil) })
		return
	}
	s.simple(done, "bash.stdin", map[string]any{"chat_id": chatID, "message_id": messageID, "text": text})
}

// StopCommand stops a running command; its card leaves once the Runner has.
func (s *Store) StopCommand(chatID, messageID string, done func(error)) {
	if s.IsMock {
		s.finishMockCommand(chatID, messageID, CommandStopped)
		if done != nil {
			s.post(func() { done(nil) })
		}
		return
	}
	s.simple(done, "bash.stop", map[string]any{"chat_id": chatID, "message_id": messageID})
}

// SendCommandToBackground sends a command the bot is waiting on to the background: the bot's call
// returns and the command runs on, out of the way of Stop in the chat.
func (s *Store) SendCommandToBackground(chatID, messageID string, done func(error)) {
	if s.IsMock {
		s.Update(messageID, chatID, func(message *Message) {
			if message.Body.Kind != BodyTool || message.Body.Tool.Run == nil {
				return
			}
			tool := *message.Body.Tool
			run := *tool.Run
			run.Background = true
			tool.IsRunning, tool.Run = false, &run
			message.Body.Tool = &tool
		})
		if done != nil {
			s.post(func() { done(nil) })
		}
		return
	}
	s.simple(done, "bash.background", map[string]any{"chat_id": chatID, "message_id": messageID})
}

// ForegroundCommands are the commands in a chat that a bot's call still waits on, which Run in
// Background sends there.
func (s *Store) ForegroundCommands(chatID string) []*Message {
	chat := s.Chat(chatID)
	if chat == nil {
		return nil
	}
	var out []*Message
	for _, message := range chat.Messages {
		if RunsInForeground(message) {
			out = append(out, message)
		}
	}
	return out
}

// finishMockCommand ends a demo command at once: the demo has no Runner.
func (s *Store) finishMockCommand(chatID, messageID string, state CommandState) {
	s.Update(messageID, chatID, func(message *Message) {
		if message.Body.Kind != BodyTool || message.Body.Tool.Run == nil {
			return
		}
		tool := *message.Body.Tool
		run := *tool.Run
		run.State, run.Prompt = state, ""
		tool.Run = &run
		message.Body.Tool = &tool
	})
}

// MARK: - Routines

// SetRoutineEnabled pauses or resumes a routine. A resumed schedule counts from now.
func (s *Store) SetRoutineEnabled(id string, enabled bool) {
	routine := s.Routine(id)
	if routine == nil {
		return
	}
	routine.IsEnabled, routine.PausedReason = enabled, ""
	routine.State = "on"
	if !enabled {
		routine.State, routine.NextRunAt = "paused", time.Time{}
	}
	s.emit(Event{Kind: EventRosterChanged})
	s.perform("routines.update", map[string]any{"id": id, "enabled": enabled})
}

// RunRoutine runs the routine now, on its bot's Runner.
func (s *Store) RunRoutine(id string) {
	routine := s.Routine(id)
	if routine == nil {
		return
	}
	routine.IsRunning = true
	s.emit(Event{Kind: EventRosterChanged})
	s.perform("routines.run", map[string]any{"id": id})
}

func (s *Store) DeleteRoutine(id string) {
	s.Routines = slices.DeleteFunc(slices.Clone(s.Routines), func(routine *Routine) bool { return routine.ID == id })
	s.emit(Event{Kind: EventRosterChanged})
	s.perform("routines.delete", map[string]any{"id": id})
}

// BotMemory reads a bot's memory from its Runner. It answers with Here false when the bot runs on
// another Device, whose disk this computer cannot read.
func (s *Store) BotMemory(id string, done func(BotMemory, error)) {
	if s.IsMock {
		memory := BotMemory{
			BotID: id, Here: true, Runner: "This computer", Path: "~/.lorca/workspaces/" + id,
			Text: "- 2026-09-10 · from your chat with the user · the user prefers short replies\n- 2026-09-12 · invoices are reconciled on Mondays\n",
			Hash: "mock", Lines: 2, Bytes: 128, MaxLines: 200, MaxBytes: 24_000,
			Topics: []string{"clients.md"}, Logs: []string{"2026-09-12", "2026-09-15"},
		}
		s.post(func() { done(memory, nil) })
		return
	}
	Async(s, func() (BotMemory, error) {
		wire, err := call[WireBotMemory](s, "bots.memory", map[string]any{"bot_id": id})
		return ToBotMemory(wire), err
	}, done)
}

// WriteBotMemory replaces the bot's MEMORY.md. With an expected hash, the write is refused when
// the file changed since it was read, so an edit never silently overwrites what the bot wrote.
func (s *Store) WriteBotMemory(id, text, expectedHash string, done func(string, error)) {
	if s.IsMock {
		s.post(func() { done("mock", nil) })
		return
	}
	params := map[string]any{"bot_id": id, "text": text}
	if expectedHash != "" {
		params["expected_hash"] = expectedHash
	}
	Async(s, func() (string, error) {
		reply, err := call[struct {
			Hash string `json:"hash"`
		}](s, "bots.memory.write", params)
		return reply.Hash, err
	}, done)
}

func (s *Store) RetryNote(chatID string) string { return s.retryNotes[chatID] }

func (s *Store) IsThinking(botID, chatID string) bool { return s.thinkingBots[chatID] == botID }

func (s *Store) DeleteChat(id string) {
	chat := s.Chat(id)
	if chat == nil {
		return
	}
	// A bot owns its DM, so deleting that row deletes the bot as one roster operation. Groups keep
	// their other members; a group with nobody left is removed too.
	if chat.IsDM() && len(chat.BotIDs) > 0 && s.Bot(chat.BotIDs[0]) != nil {
		botID := chat.BotIDs[0]
		removed := map[string]bool{}
		var changed []string
		var chats []*Chat
		for _, existing := range s.Chats {
			if !slices.Contains(existing.BotIDs, botID) {
				chats = append(chats, existing)
				continue
			}
			if s.replies != nil {
				s.replies.cancel(existing.ID)
			}
			if existing.IsDM() {
				removed[existing.ID] = true
				continue
			}
			botIDs := slices.DeleteFunc(slices.Clone(existing.BotIDs), func(member string) bool { return member == botID })
			if len(botIDs) == 0 {
				removed[existing.ID] = true
				continue
			}
			existing.BotIDs = botIDs
			changed = append(changed, existing.ID)
			chats = append(chats, existing)
		}
		s.Chats = chats
		s.Bots = slices.DeleteFunc(slices.Clone(s.Bots), func(bot *Bot) bool { return bot.ID == botID })
		s.Routines = slices.DeleteFunc(slices.Clone(s.Routines), func(routine *Routine) bool { return routine.BotID == botID })
		s.runningJobs = slices.DeleteFunc(s.runningJobs, func(job runningJob) bool {
			gone := job.botID == botID || removed[job.chatID]
			if gone {
				delete(s.jobStarts, job.id)
			}
			return gone
		})
		for chatID := range removed {
			delete(s.retryNotes, chatID)
		}
		for chatID, thinking := range s.thinkingBots {
			if thinking == botID || removed[chatID] {
				delete(s.thinkingBots, chatID)
			}
		}
		s.emit(Event{Kind: EventRosterChanged})
		for _, chatID := range changed {
			s.emit(Event{Kind: EventChatChanged, ChatID: chatID})
		}
		s.emit(Event{Kind: EventChatsChanged})
		s.perform("bots.delete", map[string]any{"id": botID})
		return
	}

	if s.replies != nil {
		s.replies.cancel(id)
	}
	s.Chats = slices.DeleteFunc(slices.Clone(s.Chats), func(c *Chat) bool { return c.ID == id })
	s.runningJobs = slices.DeleteFunc(s.runningJobs, func(job runningJob) bool {
		if job.chatID == id {
			delete(s.jobStarts, job.id)
			return true
		}
		return false
	})
	delete(s.retryNotes, id)
	delete(s.thinkingBots, id)
	s.emit(Event{Kind: EventChatsChanged})
	s.perform("chats.delete", map[string]any{"chat_id": id})
}

func (s *Store) TogglePin(id string) {
	chat := s.Chat(id)
	if chat == nil {
		return
	}
	chat.IsPinned = !chat.IsPinned
	s.sortChats()
	s.emit(Event{Kind: EventChatsChanged})
	s.perform("chats.pin", map[string]any{"chat_id": id, "pinned": chat.IsPinned})
}

// LoadOlderMessages asks the CLI for the page of messages before the chat's first one. The
// transcript calls this as it nears the top; one request per chat at a time.
func (s *Store) LoadOlderMessages(id string) {
	chat := s.Chat(id)
	if s.IsMock || s.loadingOlder[id] || chat == nil || !chat.HasMore || len(chat.Messages) == 0 {
		return
	}
	first := chat.Messages[0].ID
	s.loadingOlder[id] = true
	Async(s, func() (WireMessagePage, error) {
		return call[WireMessagePage](s, "chats.messages", map[string]any{"chat_id": id, "before": first})
	}, func(page WireMessagePage, err error) {
		delete(s.loadingOlder, id)
		current := s.Chat(id)
		if err != nil || current == nil || len(current.Messages) == 0 || current.Messages[0].ID != first {
			return
		}
		known := map[string]bool{}
		for _, message := range current.Messages {
			known[message.ID] = true
		}
		var older []*Message
		for _, wire := range page.Messages {
			if message := ToMessage(wire); !known[message.ID] {
				older = append(older, message)
			}
		}
		current.Messages = append(older, current.Messages...)
		current.HasMore = page.HasMore
		for _, message := range older {
			s.noteCommand(message, id)
		}
		s.emit(Event{Kind: EventOlderMessagesLoaded, ChatID: id})
	})
}

// IsLoadingOlder is a page of older messages on its way for the chat.
func (s *Store) IsLoadingOlder(id string) bool { return s.loadingOlder[id] }

// SearchChats answers full-text chat and message matches from the local SQLite index.
func (s *Store) SearchChats(query string, done func(WireSearchResults, error)) {
	if s.IsMock {
		s.post(func() { done(WireSearchResults{}, nil) })
		return
	}
	Async(s, func() (WireSearchResults, error) {
		return call[WireSearchResults](s, "chats.search", map[string]any{"query": query, "limit": 24})
	}, done)
}

// SetWatchedChat tells the CLI which chat the user is looking at ("" when none, or the app is not
// in front), so a reply they watch arrive is not pushed to their phone.
func (s *Store) SetWatchedChat(id string) {
	if s.IsMock || !s.IsConnected || (s.reportedWatchedChat != nil && *s.reportedWatchedChat == id) {
		return
	}
	s.reportedWatchedChat = &id
	var chatID any
	if id != "" {
		chatID = id
	}
	go func() { _ = s.request("ui.watching", map[string]any{"chat_id": chatID}, nil) }()
}

func (s *Store) MarkRead(id string) {
	chat := s.Chat(id)
	if chat == nil || chat.UnreadCount <= 0 {
		return
	}
	chat.UnreadCount = 0
	s.emit(Event{Kind: EventChatsChanged})
	s.perform("chats.mark_read", map[string]any{"chat_id": id})
}

func (s *Store) Rename(id, title string) {
	chat := s.Chat(id)
	if chat == nil || !chat.IsGroup() {
		return
	}
	chat.CustomTitle = strings.TrimSpace(title)
	s.emit(Event{Kind: EventChatChanged, ChatID: id})
	s.emit(Event{Kind: EventChatsChanged})
	s.perform("chats.rename", map[string]any{"chat_id": id, "title": chat.CustomTitle})
}

// SetDescription is what a group is for; every member reads it in its system prompt.
func (s *Store) SetDescription(text, chatID string) {
	chat := s.Chat(chatID)
	trimmed := strings.TrimSpace(text)
	if chat == nil || !chat.IsGroup() || chat.GroupDescription == trimmed {
		return
	}
	chat.GroupDescription = trimmed
	s.emit(Event{Kind: EventChatChanged, ChatID: chatID})
	s.perform("chats.set_description", map[string]any{"chat_id": chatID, "description": trimmed})
}

func (s *Store) AddBot(botID, chatID string) {
	chat := s.Chat(chatID)
	if chat == nil || !chat.CanAddBot() || slices.Contains(chat.BotIDs, botID) {
		return
	}
	chat.BotIDs = append(slices.Clone(chat.BotIDs), botID)
	if s.IsMock {
		name := s.botName(botID, "A bot")
		s.Append(&Message{ID: NewMessageID(), Author: System, Body: Body{Kind: BodyNotice, Text: name + " joined the chat."}, CreatedAt: time.Now()}, chatID)
	}
	s.emit(Event{Kind: EventChatChanged, ChatID: chatID})
	s.emit(Event{Kind: EventChatsChanged})
	s.perform("chats.add_bot", map[string]any{"chat_id": chatID, "bot_id": botID})
}

func (s *Store) RemoveBot(botID, chatID string) {
	chat := s.Chat(chatID)
	if chat == nil || !chat.CanRemoveBot() {
		return
	}
	chat.BotIDs = slices.DeleteFunc(slices.Clone(chat.BotIDs), func(member string) bool { return member == botID })
	s.emit(Event{Kind: EventChatChanged, ChatID: chatID})
	s.emit(Event{Kind: EventChatsChanged})
	s.perform("chats.remove_bot", map[string]any{"chat_id": chatID, "bot_id": botID})
}

// SetOwner makes a group member the bot holding the work.
func (s *Store) SetOwner(botID, chatID string) {
	chat := s.Chat(chatID)
	if chat == nil || !chat.IsGroup() || !slices.Contains(chat.BotIDs, botID) || chat.Owner() == botID {
		return
	}
	chat.OwnerBotID = botID
	s.emit(Event{Kind: EventChatChanged, ChatID: chatID})
	s.perform("chats.set_owner", map[string]any{"chat_id": chatID, "bot_id": botID})
}

// MARK: - Messages

func (s *Store) Append(message *Message, chatID string) string {
	chat := s.Chat(chatID)
	if chat == nil {
		return ""
	}
	chat.Messages = append(slices.Clone(chat.Messages), message)
	s.noteCommand(message, chatID)
	s.emit(Event{Kind: EventMessageAdded, ChatID: chatID, MessageID: message.ID})
	s.sortChats()
	s.emit(Event{Kind: EventChatsChanged})
	return message.ID
}

// Update changes a message in place. It is called for every streamed step, so it stays off the
// chat-list path; the sidebar preview refreshes when a step finishes instead.
func (s *Store) Update(messageID, chatID string, transform func(*Message)) {
	chat := s.Chat(chatID)
	if chat == nil {
		return
	}
	index := slices.IndexFunc(chat.Messages, func(m *Message) bool { return m.ID == messageID })
	if index < 0 {
		return
	}
	updated := *chat.Messages[index]
	transform(&updated)
	messages := slices.Clone(chat.Messages)
	messages[index] = &updated
	chat.Messages = messages
	s.noteCommand(&updated, chatID)
	s.emit(Event{Kind: EventMessageChanged, ChatID: chatID, MessageID: messageID})
}

func (s *Store) RefreshChatList() { s.emit(Event{Kind: EventChatsChanged}) }

// Send sends the message and returns the chat it landed in. `mentions` are the bots picked from the
// `@` menu, which the CLI hands the bot by id; `replyTo` is the message the user answers, which the
// bot reads quoted.
func (s *Store) Send(text string, attachments []OutgoingAttachment, mentions []string, chatID, replyTo string) string {
	trimmed := strings.TrimSpace(text)
	chat := s.Chat(chatID)
	if (trimmed == "" && len(attachments) == 0) || chat == nil {
		return chatID
	}
	var quote *ReplyQuote
	if replyTo != "" {
		if original := chat.Message(replyTo); original != nil {
			quote = QuoteOf(original)
		}
	}
	// The files are known here already; the CLI keeps the ids the bubble shows.
	files := make([]Attachment, 0, len(attachments))
	for _, outgoing := range attachments {
		s.attachmentFiles[outgoing.Attachment.ID] = outgoing.Path
		files = append(files, outgoing.Attachment)
	}
	message := &Message{
		ID:          NewMessageID(),
		Author:      You,
		Body:        Body{Kind: BodyText, Text: trimmed},
		CreatedAt:   time.Now(),
		Attachments: files,
		ReplyTo:     quote,
	}
	s.Append(message, chatID)

	if s.IsMock {
		if s.replies != nil {
			s.replies.respond(trimmed, chat, message.ID)
		}
		return chatID
	}

	// Expect a turn to start; the CLI's job events confirm or clear this. A DM names its bot right
	// away so the working row appears with the send.
	if len(chat.BotIDs) > 0 {
		pendingID := "pending:" + chatID
		botID := ""
		if chat.IsDM() {
			botID = chat.BotIDs[0]
		}
		s.runningJobs = append(s.runningJobs, runningJob{id: pendingID, chatID: chatID, botID: botID})
		s.emit(Event{Kind: EventRespondingChanged, ChatID: chatID})
		s.emit(Event{Kind: EventChatsChanged})
		s.later(4*time.Second, func() {
			if !slices.ContainsFunc(s.runningJobs, func(job runningJob) bool { return job.id == pendingID }) {
				return
			}
			// Nothing started (no Runner answered); stop showing the bot at work.
			s.runningJobs = slices.DeleteFunc(s.runningJobs, func(job runningJob) bool { return job.id == pendingID })
			s.emit(Event{Kind: EventRespondingChanged, ChatID: chatID})
			s.emit(Event{Kind: EventChatsChanged})
		})
	}
	wireAttachments := make([]map[string]any, 0, len(attachments))
	for _, outgoing := range attachments {
		entry := map[string]any{"id": outgoing.Attachment.ID, "path": outgoing.Path, "name": outgoing.Attachment.Name, "mime": outgoing.Attachment.Mime, "width": nil, "height": nil}
		if outgoing.Attachment.Width > 0 {
			entry["width"], entry["height"] = outgoing.Attachment.Width, outgoing.Attachment.Height
		}
		wireAttachments = append(wireAttachments, entry)
	}
	if mentions == nil {
		mentions = []string{}
	}
	params := map[string]any{"chat_id": chatID, "text": trimmed, "message_id": message.ID, "mentions": mentions, "attachments": wireAttachments}
	if quote != nil {
		params["reply_to"] = quote.MessageID
	}
	s.perform("chats.send", params)
	return chatID
}

// MARK: - Attachments

// AvatarPath is where the bot's profile image is once this computer has it. The first ask for one
// that is not here fetches the blob and redraws the roster when it lands; until then callers draw
// the symbol and accent.
func (s *Store) AvatarPath(bot *Bot) string {
	if bot == nil || bot.Avatar == nil {
		return ""
	}
	if path, ok := s.attachmentFiles[bot.Avatar.ID]; ok {
		return path
	}
	botID := bot.ID
	s.fetchAttachment(*bot.Avatar, func() { s.rosterTouched(botID) })
	return ""
}

// LocalFile is where an attachment's bytes are on this computer. A file sent from here is known at
// once; one sent from another Device is fetched through the CLI, and the message reloads when it
// lands.
func (s *Store) LocalFile(attachment Attachment, chatID, messageID string) string {
	if path, ok := s.attachmentFiles[attachment.ID]; ok {
		return path
	}
	s.fetchAttachment(attachment, func() { s.emit(Event{Kind: EventMessageChanged, ChatID: chatID, MessageID: messageID}) })
	return ""
}

func (s *Store) fetchAttachment(attachment Attachment, landed func()) {
	if s.IsMock || s.fetchingAttachment[attachment.ID] || s.attachmentErrors[attachment.ID] != "" {
		return
	}
	s.fetchingAttachment[attachment.ID] = true
	identity := s.IdentityID
	Async(s, func() (string, error) {
		reply, err := call[struct {
			Path string `json:"path"`
		}](s, "files.path", map[string]any{"attachment": map[string]any{"id": attachment.ID, "name": attachment.Name, "mime": attachment.Mime, "size": attachment.Size}})
		if err == nil && reply.Path == "" {
			err = &RequestError{L("File unavailable")}
		}
		return reply.Path, err
	}, func(path string, err error) {
		if identity != s.IdentityID {
			return
		}
		delete(s.fetchingAttachment, attachment.ID)
		if err != nil {
			s.attachmentErrors[attachment.ID] = ErrorText(err)
			log.Printf("fetching %s failed: %s", attachment.Name, ErrorText(err))
			landed()
			return
		}
		s.attachmentFiles[attachment.ID] = path
		landed()
	})
}

func (s *Store) IsResponding(chatID string) bool {
	return slices.ContainsFunc(s.runningJobs, func(job runningJob) bool { return job.chatID == chatID })
}

// RunningCommands are the chat's running tasks: its commands running in their terminals, here or
// on their Runners, in the order they started, once each has run for TaskDelay.
func (s *Store) RunningCommands(chatID string) []*Message {
	chat := s.Chat(chatID)
	if chat == nil {
		return nil
	}
	now := time.Now()
	var out []*Message
	for _, message := range chat.Messages {
		run := CommandRunOf(message)
		if run == nil || !run.TakesInput() {
			continue
		}
		// One that was running before this app heard of it has run long enough.
		if now.Sub(s.commandStarts[message.ID]) >= TaskDelay {
			out = append(out, message)
		}
	}
	return out
}

// noteCommand notes when a command starts running in its terminal, and tells the observers once
// it has run for TaskDelay.
func (s *Store) noteCommand(message *Message, chatID string) {
	run := CommandRunOf(message)
	if run == nil || !run.TakesInput() {
		delete(s.commandStarts, message.ID)
		return
	}
	if _, ok := s.commandStarts[message.ID]; ok {
		return
	}
	s.commandStarts[message.ID] = time.Now()
	s.later(TaskDelay, func() { s.emit(Event{Kind: EventRunningTasksChanged, ChatID: chatID}) })
}

// WorkingBots are the bots with a turn running in this chat, in the order they started.
func (s *Store) WorkingBots(chatID string) []string {
	var seen []string
	for _, job := range s.runningJobs {
		if job.chatID == chatID && job.botID != "" && !slices.Contains(seen, job.botID) {
			seen = append(seen, job.botID)
		}
	}
	return seen
}

// IsWorking is whether the bot has a turn running anywhere: the green dot on its avatar.
func (s *Store) IsWorking(botID string) bool {
	return slices.ContainsFunc(s.runningJobs, func(job runningJob) bool { return job.botID == botID })
}

// setMockWorking is the mock reply engine's turns, so the demo shows the same working state as
// the CLI.
func (s *Store) setMockWorking(botID, chatID string, working bool) {
	id := "mock:" + chatID + ":" + botID
	s.runningJobs = slices.DeleteFunc(s.runningJobs, func(job runningJob) bool { return job.id == id })
	if working {
		s.runningJobs = append(s.runningJobs, runningJob{id: id, chatID: chatID, botID: botID})
	}
	s.emit(Event{Kind: EventRespondingChanged, ChatID: chatID})
	s.emit(Event{Kind: EventChatsChanged})
}

// SendNow has the bot's turn read a message it holds for its next step now: a command it waits on
// goes to the background, and a reply in progress stops where it got to.
func (s *Store) SendNow(messageID, chatID string) {
	if s.IsMock {
		if s.replies != nil {
			s.replies.sendNow(chatID)
		}
		return
	}
	s.perform("chats.send_now", map[string]any{"chat_id": chatID, "message_id": messageID})
}

func (s *Store) setMockQueued(messageID, chatID string, queued bool) {
	s.Update(messageID, chatID, func(message *Message) { message.Queued = queued })
}

func (s *Store) StopResponding(chatID string) {
	if s.IsMock {
		if s.replies != nil {
			s.replies.cancel(chatID)
		}
		return
	}
	s.perform("chats.stop", map[string]any{"chat_id": chatID})
}

// MARK: - Identity, pairing, providers

func (s *Store) setIdentity(has bool) {
	s.HasIdentity = &has
	s.emit(Event{Kind: EventIdentityChanged})
}

// CreateIdentity makes a new account here and answers its backup phrase.
func (s *Store) CreateIdentity(done func([]string, error)) {
	Async(s, func() ([]string, error) {
		created, err := call[struct {
			Phrase []string `json:"phrase"`
		}](s, "identity.create", nil)
		return created.Phrase, err
	}, func(phrase []string, err error) {
		if err == nil {
			s.IsIdentityDevice = true
			s.setIdentity(true)
		}
		done(phrase, err)
	})
}

// UnpairDevice unpairs another Device. The CLI has the relay drop its key; the Device wipes its
// copy of the account the next time it connects. Fails when the relay could not be told.
func (s *Store) UnpairDevice(id string, done func(error)) {
	finish := func(err error) {
		if err == nil {
			s.Devices = slices.DeleteFunc(slices.Clone(s.Devices), func(device *Device) bool { return device.ID == id })
			s.emit(Event{Kind: EventRosterChanged})
		}
		done(err)
	}
	s.simple(finish, "device.unpair", map[string]any{"id": id})
}

// UpdateDevice has a self-updating CLI, this Device's or another's through the relay, install the
// latest release, which it restarts into once no bot is at work there.
func (s *Store) UpdateDevice(id string, done func(error)) {
	if s.IsMock {
		if device := s.Device(id); device != nil && device.Update != nil {
			update := *device.Update
			update.State = "restarting"
			device.Update = &update
			s.emit(Event{Kind: EventRosterChanged})
		}
		s.post(func() { done(nil) })
		return
	}
	s.simple(done, "device.update", map[string]any{"id": id})
}

// ForgetIdentity unpairs this Device. The CLI asks the relay to drop its key, best effort, then
// forgets the identity here; the `identity.changed` it sends brings back onboarding.
func (s *Store) ForgetIdentity(done func(error)) {
	Async(s, func() (struct{}, error) { return struct{}{}, s.request("identity.forget", nil, nil) }, func(_ struct{}, err error) { done(err) })
}

// DeleteAccount deletes the account on the relay and on this Device; the other Devices forget it
// as the relay drops them. Fails when the relay could not be told, and nothing is deleted then.
func (s *Store) DeleteAccount(done func(error)) {
	Async(s, func() (struct{}, error) { return struct{}{}, s.request("identity.delete", nil, nil) }, func(_ struct{}, err error) { done(err) })
}

func (s *Store) RestoreIdentity(phrase string, done func(error)) {
	Async(s, func() (struct{}, error) {
		return struct{}{}, s.request("identity.restore", map[string]any{"phrase": phrase}, nil)
	}, func(_ struct{}, err error) {
		if err == nil {
			s.IsIdentityDevice = true
			s.setIdentity(true)
		}
		done(err)
	})
}

func (s *Store) StartPairing(done func(WirePairStart, error)) {
	if s.IsMock {
		start := WirePairStart{Nonce: "482913", PairingString: "lorca://pair?relay=https%3A%2F%2Florca.app&id=idk_9f2c41ab&ek=ek_57ca0d3b&n=482913"}
		s.post(func() { done(start, nil) })
		return
	}
	Async(s, func() (WirePairStart, error) { return call[WirePairStart](s, "pair.start", nil) }, done)
}

func (s *Store) PairingStatus(nonce string, done func(WirePairStatus, error)) {
	if s.IsMock {
		s.post(func() { done(WirePairStatus{State: "waiting"}, nil) })
		return
	}
	Async(s, func() (WirePairStatus, error) {
		return call[WirePairStatus](s, "pair.status", map[string]any{"nonce": nonce})
	}, done)
}

func (s *Store) CancelPairing(nonce string) { s.perform("pair.cancel", map[string]any{"nonce": nonce}) }

func (s *Store) AcceptPairing(pairingString string, done func(error)) {
	Async(s, func() (struct{}, error) {
		return struct{}{}, s.request("pair.accept", map[string]any{"pairing_string": pairingString}, nil)
	}, func(_ struct{}, err error) {
		if err == nil {
			s.setIdentity(true)
		}
		done(err)
	})
}

// AbortPairing stops waiting on the other Device: AcceptPairing fails with "Pairing cancelled".
func (s *Store) AbortPairing() { s.perform("pair.abort", nil) }

// AccountHasProvider answers whether the account this computer just joined has a provider
// connected. The CLI answers once its first pull from the relay has brought the account's
// credentials, or after half a minute (`sync.account`).
func (s *Store) AccountHasProvider(done func(bool)) {
	Async(s, func() (WireSyncAccount, error) { return call[WireSyncAccount](s, "sync.account", nil) }, func(synced WireSyncAccount, err error) {
		providers := s.Providers
		if err == nil {
			// A CLI that cannot answer (an older one, or one that restarted) leaves what the store has.
			providers = ToProviders(synced.Providers)
		}
		done(slices.ContainsFunc(providers, func(p ProviderCredential) bool { return p.IsConnected }))
	})
}

// SavedKey is the saved key of an API-key provider or a custom one, for its sheet.
type SavedKey struct {
	APIKey  *string `json:"api_key"`
	BaseURL *string `json:"base_url"`
}

func (s *Store) ProviderAPIKey(kind ProviderKind, done func(SavedKey, error)) {
	if s.IsMock {
		s.post(func() { done(SavedKey{}, nil) })
		return
	}
	Async(s, func() (SavedKey, error) { return call[SavedKey](s, "providers.api_key", map[string]any{"kind": kind}) }, done)
}

// ConnectAPIKey connects an API-key provider. An empty base URL means the provider's own API.
func (s *Store) ConnectAPIKey(kind ProviderKind, apiKey, baseURL string, done func(error)) {
	params := map[string]any{"api_key": apiKey}
	if trimmed := strings.TrimSpace(baseURL); trimmed != "" {
		params["base_url"] = trimmed
	}
	Async(s, func() (struct{}, error) { return struct{}{}, s.request(ConnectMethod(kind), params, nil) }, func(_ struct{}, err error) { done(err) })
}

// ConnectSignIn runs a subscription sign-in (`providers.connect_chatgpt`, `providers.connect_grok`):
// the CLI opens the browser on this computer and the tokens go to the whole account.
func (s *Store) ConnectSignIn(kind ProviderKind, done func(error)) {
	Async(s, func() (struct{}, error) { return struct{}{}, s.request(ConnectMethod(kind), nil, nil) }, func(_ struct{}, err error) { done(err) })
}

// CancelSignIn stops a sign-in that is waiting on the browser.
func (s *Store) CancelSignIn() { s.perform("providers.auth.cancel", nil) }

func (s *Store) Credential(kind ProviderKind) *ProviderCredential {
	for i := range s.Providers {
		if s.Providers[i].Kind == kind {
			return &s.Providers[i]
		}
	}
	return nil
}

// DisconnectProvider disconnects a provider for the whole account, or deletes a custom one.
func (s *Store) DisconnectProvider(kind ProviderKind, done func(error)) {
	if s.IsMock && IsCustomKind(kind) {
		s.Providers = slices.DeleteFunc(slices.Clone(s.Providers), func(p ProviderCredential) bool { return p.Kind == kind })
		s.rosterTouched("")
		s.post(func() { done(nil) })
		return
	}
	Async(s, func() (struct{}, error) {
		return struct{}{}, s.request("providers.disconnect", map[string]any{"kind": kind}, nil)
	}, func(_ struct{}, err error) { done(err) })
}

// ModelQuery is the server ListCustomModels asks.
type ModelQuery struct {
	Name    string
	API     CustomAPI
	BaseURL string
	APIKey  string
}

// ListCustomModels answers the models a custom provider's server lists that its protocol can run,
// in its order (`providers.list_models`), for the sheet to pick from. Listed is false when the
// server publishes no list. Fails with why the server could not be asked.
func (s *Store) ListCustomModels(query ModelQuery, done func(models []CustomModel, listed bool, err error)) {
	if s.IsMock {
		s.later(300*time.Millisecond, func() {
			models, listed := mockListedModels(query.BaseURL)
			done(models, listed, nil)
		})
		return
	}
	Async(s, func() (WireModelList, error) {
		return call[WireModelList](s, "providers.list_models", map[string]any{
			"name": query.Name, "api": string(query.API), "base_url": strings.TrimSpace(query.BaseURL), "api_key": query.APIKey,
		})
	}, func(reply WireModelList, err error) {
		var models []CustomModel
		for _, model := range reply.Models {
			models = append(models, ToCustomModel(model))
		}
		done(models, reply.Listed, err)
	})
}

// CustomProvider is what SaveCustomProvider saves.
type CustomProvider struct {
	// Kind is the provider to save over; empty adds one.
	Kind    ProviderKind
	Name    string
	API     CustomAPI
	BaseURL string
	APIKey  string
	// Models are the ids it offers, the default first; empty takes every model the server lists.
	Models []string
}

// SaveCustomProvider adds a custom provider, or saves the one Kind names, once the CLI has heard
// from its server (`providers.connect_custom`). It answers the provider's kind.
func (s *Store) SaveCustomProvider(options CustomProvider, done func(ProviderKind, error)) {
	name := strings.TrimSpace(options.Name)
	baseURL := strings.TrimSpace(options.BaseURL)
	models := []string{}
	for _, id := range options.Models {
		if trimmed := strings.TrimSpace(id); trimmed != "" {
			models = append(models, trimmed)
		}
	}
	if s.IsMock {
		kind := options.Kind
		if kind == "" {
			kind = "custom:" + strings.ReplaceAll(strings.ToLower(name), " ", "-")
		}
		saved := ProviderCredential{Kind: kind, IsConnected: true, Detail: baseURL, BaseURL: baseURL, Name: name, API: options.API}
		if len(models) > 0 {
			saved.ReviewModel = models[0]
		}
		for _, id := range models {
			// A decision model does not think out loud.
			var levels []string
			if !options.API.Decides() {
				levels = []string{"low", "medium", "high"}
			}
			saved.Models = append(saved.Models, CustomModel{ID: id, Levels: levels})
		}
		if existing := s.Credential(kind); existing != nil {
			*existing = saved
		} else {
			s.Providers = append(slices.Clone(s.Providers), saved)
		}
		s.rosterTouched("")
		s.post(func() { done(kind, nil) })
		return
	}
	params := map[string]any{"name": name, "api": string(options.API), "base_url": baseURL, "api_key": strings.TrimSpace(options.APIKey), "models": models}
	if options.Kind != "" {
		params["kind"] = options.Kind
	}
	Async(s, func() (ProviderKind, error) {
		saved, err := call[struct {
			Kind string `json:"kind"`
		}](s, "providers.connect_custom", params)
		if err != nil {
			return "", err
		}
		if IsCustomKind(saved.Kind) {
			return saved.Kind, nil
		}
		return "custom:" + saved.Kind, nil
	}, done)
}

func (s *Store) SetRelayURL(url string) {
	s.RelayURL = url
	s.perform("config.set", map[string]any{"relay_url": url})
}

// ResetMockData replays the seeded conversations so the demo can be restarted from the Debug menu.
func (s *Store) ResetMockData() {
	if !s.IsMock {
		return
	}
	s.Reviews = mockReviews()
	if s.replies != nil {
		for _, chat := range s.Chats {
			s.replies.cancel(chat.ID)
		}
	}
	s.mockFeedback = map[string]BotFeedback{}
	s.Devices = mockDevices()
	s.mockBrowser = nil
	s.Bots = mockBots()
	s.Chats = mockChats()
	s.Routines = mockRoutines()
	s.mockPlaybooks = mockPlaybooks()
	s.Playbooks = nil
	for _, record := range s.mockPlaybooks {
		s.Playbooks = append(s.Playbooks, record.summary())
	}
	s.Budgets = mockBudgets()
	s.AutoReview = mockAutoReview()
	s.Attention = DefaultAttention()
	s.SharedLinks = mockSharedLinks()
	s.Providers = mockProviders()
	s.Models = mockModels()
	s.sortChats()
	s.emit(Event{Kind: EventSnapshotReplaced})
}

// OfflineStatus is what the offline state says while the CLI is not answering: where the launcher
// stands, or why the CLI is not running.
func (s *Store) OfflineStatus() string {
	status := s.CLI.Launcher
	switch status.Kind {
	case "probing":
		return L("Looking for the CLI…")
	case "starting":
		return L("Starting the CLI…")
	case "running":
		if status.External {
			return L("Using the CLI already running on this computer")
		}
		return L("CLI started by the app")
	case "failed":
		failure := status.Failure
		if failure == nil {
			return L("Waiting to start the CLI")
		}
		switch failure.Kind {
		case "missing_binary":
			return L("The lorca CLI is not bundled with this build and is not on PATH.")
		case "launch":
			binary := failure.Binary
			if binary == "" {
				binary = "lorca"
			}
			return L("Could not start %@: %@", binary, failure.Reason)
		case "exited":
			return L("The CLI exited with code %d. See %@.", failure.Code, failure.Log)
		case "startup_closed":
			return L("The CLI closed its startup channel before becoming ready. See %@.", failure.Log)
		}
		if failure.Reason != "" {
			return failure.Reason
		}
		return L("Waiting to start the CLI")
	}
	return L("Waiting to start the CLI")
}

// ErrNotFound is a lookup that found nothing.
var ErrNotFound = errors.New("not found")
