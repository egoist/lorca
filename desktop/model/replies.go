package model

import (
	"fmt"
	"math/rand/v2"
	"strings"
	"time"
)

// Stands in for the CLI's agent loop in the demo, after the macOS app's ReplyEngine: bots work for
// a moment, run fake tools, hand off between each other, and post whole replies. Each step waits
// on a timer and runs on the main thread, as the rest of the store does.

type stepKind int

const (
	stepThink stepKind = iota
	stepSay
	stepTool
	stepHandoff
)

type step struct {
	kind    stepKind
	botID   string
	seconds float64
	text    string
	tool    ToolInvocation
	handoff Handoff
}

type held struct {
	prompt    string
	chat      *Chat
	messageID string
}

type replyTask struct{ cancelled bool }

type replyEngine struct {
	store   *Store
	tasks   map[string]*replyTask
	working map[string]string
	// steering is messages typed during a mock turn, promoted together at its next tool or answer
	// boundary.
	steering  map[string][]held
	turnCount int
}

func newReplyEngine(store *Store) *replyEngine {
	return &replyEngine{store: store, tasks: map[string]*replyTask{}, working: map[string]string{}, steering: map[string][]held{}}
}

func (e *replyEngine) cancel(chatID string) {
	if task := e.tasks[chatID]; task != nil {
		task.cancelled = true
	}
	delete(e.tasks, chatID)
	for _, message := range e.steering[chatID] {
		e.store.setMockQueued(message.messageID, chatID, false)
	}
	delete(e.steering, chatID)
	e.setWorking("", chatID)
}

// sendNow drops the turn's step and answers the messages it holds.
func (e *replyEngine) sendNow(chatID string) {
	task := e.tasks[chatID]
	if task == nil {
		return
	}
	prompt, chat, ok := e.takeSteering(chatID)
	if !ok {
		return
	}
	task.cancelled = true
	delete(e.tasks, chatID)
	e.start(prompt, chat)
}

func (e *replyEngine) setWorking(botID, chatID string) {
	if previous := e.working[chatID]; previous != "" && previous != botID {
		e.store.setMockWorking(previous, chatID, false)
	}
	if botID != "" {
		e.working[chatID] = botID
		e.store.setMockWorking(botID, chatID, true)
	} else {
		delete(e.working, chatID)
	}
}

func (e *replyEngine) respond(prompt string, chat *Chat, messageID string) {
	if e.tasks[chat.ID] != nil {
		e.steering[chat.ID] = append(e.steering[chat.ID], held{prompt, chat, messageID})
		e.store.setMockQueued(messageID, chat.ID, true)
		return
	}
	e.start(prompt, chat)
}

func (e *replyEngine) start(prompt string, chat *Chat) {
	script := e.makeScript(prompt, chat)
	if len(script) == 0 {
		return
	}
	e.turnCount++
	task := &replyTask{}
	e.tasks[chat.ID] = task
	e.next(script, chat.ID, task)
}

// next runs the script's first step, then the rest once it is done.
func (e *replyEngine) next(steps []step, chatID string, task *replyTask) {
	if task.cancelled {
		return
	}
	if len(steps) == 0 {
		delete(e.tasks, chatID)
		e.setWorking("", chatID)
		return
	}
	current, rest := steps[0], steps[1:]
	e.run(current, chatID, task, func() {
		if task.cancelled {
			return
		}
		if current.kind != stepThink {
			if prompt, chat, ok := e.takeSteering(chatID); ok {
				e.turnCount++
				rest = e.makeScript(prompt, chat)
			}
		}
		e.next(rest, chatID, task)
	})
}

func (e *replyEngine) takeSteering(chatID string) (string, *Chat, bool) {
	queued := e.steering[chatID]
	delete(e.steering, chatID)
	if len(queued) == 0 {
		return "", nil, false
	}
	prompts := make([]string, 0, len(queued))
	for _, message := range queued {
		e.store.setMockQueued(message.messageID, chatID, false)
		prompts = append(prompts, message.prompt)
	}
	return strings.Join(prompts, "\n"), queued[len(queued)-1].chat, true
}

func (e *replyEngine) after(seconds float64, fn func()) {
	e.store.later(time.Duration(seconds*float64(time.Second)), fn)
}

func (e *replyEngine) run(s step, chatID string, task *replyTask, done func()) {
	switch s.kind {
	case stepThink:
		e.setWorking(s.botID, chatID)
		e.after(s.seconds, done)
	case stepSay:
		e.setWorking(s.botID, chatID)
		e.after(float64(len(s.text))*0.004, func() {
			if task.cancelled {
				return
			}
			e.store.Append(&Message{ID: NewMessageID(), Author: BotAuthor(s.botID), Body: textBody(s.text), CreatedAt: time.Now()}, chatID)
			e.setWorking("", chatID)
			done()
		})
	case stepTool:
		e.setWorking(s.botID, chatID)
		running := s.tool
		running.IsRunning = true
		id := e.store.Append(&Message{ID: NewMessageID(), Author: BotAuthor(s.botID), Body: Body{Kind: BodyTool, Tool: &running}, State: MessageState{Kind: StateStreaming}, CreatedAt: time.Now()}, chatID)
		if id == "" {
			done()
			return
		}
		e.after(s.seconds, func() {
			finished := s.tool
			finished.IsRunning = false
			e.store.Update(id, chatID, func(message *Message) {
				message.Body = Body{Kind: BodyTool, Tool: &finished}
				message.State = MessageState{Kind: StateComplete}
			})
			e.store.RefreshChatList()
			done()
		})
	case stepHandoff:
		e.store.Append(&Message{ID: NewMessageID(), Author: BotAuthor(s.handoff.From), Body: Body{Kind: BodyHandoff, Handoff: s.handoff}, CreatedAt: time.Now()}, chatID)
		e.after(0.6, done)
	}
}

func between(low, high float64) float64 { return low + rand.Float64()*(high-low) }

func pick(items ...string) string { return items[rand.IntN(len(items))] }

func (e *replyEngine) makeScript(prompt string, chat *Chat) []step {
	members := e.store.BotsIn(chat)
	if len(members) == 0 {
		return nil
	}
	lowered := strings.ToLower(prompt)
	var mentioned []*Bot
	for _, bot := range members {
		if strings.Contains(lowered, "@"+strings.ToLower(bot.Name)) {
			mentioned = append(mentioned, bot)
		}
	}
	everyone := strings.Contains(lowered, "@everyone")
	responders := mentioned
	switch {
	case everyone:
		responders = members
	case len(mentioned) == 0:
		responders = members[:1]
	}
	lead := responders[0]

	// A bot on an offline Runner cannot run its turn; the envelope waits on the relay.
	if host := e.store.Device(lead.RunnerID); host != nil && host.Status == StatusOffline {
		return []step{
			{kind: stepThink, botID: lead.ID, seconds: 0.5},
			{kind: stepSay, botID: lead.ID, text: fmt.Sprintf("I'm queued on the relay — %s is offline, so this turn runs when that Runner reconnects.", host.Name)},
		}
	}

	script := []step{{kind: stepThink, botID: lead.ID, seconds: between(0.5, 1.1)}}
	wantsWork := len(prompt) > 46 || !strings.Contains(prompt, "?")
	var helper *Bot
	for _, bot := range members {
		if bot.ID != lead.ID {
			if host := e.store.Device(bot.RunnerID); host == nil || host.Status != StatusOffline {
				helper = bot
				break
			}
		}
	}
	if chat.IsGroup() && wantsWork && helper != nil && e.turnCount%2 == 0 {
		script = append(script,
			step{kind: stepTool, botID: lead.ID, seconds: 0.7, tool: ToolInvocation{Name: "list_teammates", Summary: fmt.Sprintf("Listed %d teammates", len(members)), Detail: e.teammateDetail(members)}},
			step{kind: stepHandoff, handoff: Handoff{From: lead.ID, To: helper.ID, Reason: handoffReason(helper)}},
			step{kind: stepThink, botID: helper.ID, seconds: 0.5},
			step{kind: stepSay, botID: helper.ID, text: mockReply(helper, prompt)},
			step{kind: stepSay, botID: lead.ID, text: mockSummary(helper)},
		)
	} else {
		script = append(script, step{kind: stepSay, botID: lead.ID, text: mockReply(lead, prompt)})
	}
	if everyone {
		for _, bot := range members[1:] {
			if bot.ID == lead.ID {
				continue
			}
			script = append(script, step{kind: stepThink, botID: bot.ID, seconds: 0.4}, step{kind: stepSay, botID: bot.ID, text: mockReply(bot, prompt)})
		}
	}
	return script
}

func (e *replyEngine) teammateDetail(members []*Bot) string {
	rows := make([]string, 0, len(members))
	for _, bot := range members {
		host := e.store.Device(bot.RunnerID)
		name, status := "?", ProviderName(bot.Provider, e.store.Providers)
		if host != nil {
			name = host.Name
			if host.Status == StatusOffline {
				status = "offline"
			}
		}
		rows = append(rows, fmt.Sprintf(`  { "name": "%s", "runner": "%s", "provider": "%s" }`, bot.Name, name, status))
	}
	return "{\n  \"teammates\": [\n" + strings.Join(rows, ",\n") + "\n  ]\n}"
}

func handoffReason(bot *Bot) string {
	switch bot.ID {
	case "bot-patch":
		return "Write the change"
	case "bot-scout":
		return "Pull the context first"
	case "bot-quill":
		return "Say it in plain words"
	case "bot-ember":
		return "Check what is deployed"
	}
	return "Take the next step"
}

func mockSummary(bot *Bot) string {
	return pick(
		fmt.Sprintf("That matches what I expected from %s. I'll keep the thread here until you say otherwise.", bot.Name),
		fmt.Sprintf("%s has it. Tell me if you want that turned into a task on another Runner.", bot.Name),
		"Good — that closes the loop. Anything you want me to push back on?",
	)
}

func mockReply(bot *Bot, prompt string) string {
	lowered := strings.ToLower(prompt)
	if strings.Contains(lowered, "hello") || strings.Contains(lowered, "hi ") || lowered == "hi" {
		return "Here. Ready when you are."
	}
	switch bot.ID {
	case "bot-patch":
		return pick(
			"Smallest version that works:\n\n```rust\npub async fn serve(addr: SocketAddr) -> Result<()> {\n    let listener = TcpListener::bind(addr).await?;\n    tracing::info!(%addr, \"lorca serve\");\n    while let Ok((stream, _)) = listener.accept().await {\n        tokio::spawn(handle(stream));\n    }\n    Ok(())\n}\n```\n\nOne task per connection, and `handle` owns the decrypt step so the accept loop stays dumb.",
			"I'd keep this in the CLI rather than the app. The app should stay a renderer — the moment it knows how to decrypt, the key material has two homes and the threat model gets harder to explain.",
			"Two options, and they are not close:\n\n- Put it behind `bootstrap` and let the app render whatever comes back.\n- Add a new event kind and teach both sides about it.\n\nThe first one is free. Take the first one.",
		)
	case "bot-scout":
		return pick(
			"Checked the tree. The only place that touches this is the `blobs` handler and one test fixture, so the change is contained. Nothing in `web/` reads it.",
			"Relevant prior art: Happy wraps the account DEK to each machine public key at pairing time, which is what `ARCHITECTURE.md` already describes. Following it means recovery is the backup phrase and nothing else, which is the property you want.",
			"I found two answers and they disagree. The schema says `seq` is unique per identity; the handler treats it as unique per `(identity, kind)`. Worth deciding before the first migration lands, because it is painful afterwards.",
		)
	case "bot-quill":
		return pick(
			"Draft: \"Your bots run on computers you own. Assign one to a Runner, and it works there with your account's encrypted provider credentials. The relay carries ciphertext and nothing else.\"\n\nThree sentences, no adjectives doing work they haven't earned.",
			"I'd cut \"seamlessly\" and \"powerful\". They are the words people skim. What is left says the same thing and is shorter.",
		)
	case "bot-ember":
		return pick(
			"Blast radius first: this touches the Worker only, no D1 migration, so a bad deploy is a rollback and not a restore.",
			"Deployed. The relay is answering the challenge endpoint in about 40ms from here, which is the number to watch when we add the blob listing.",
		)
	}
	return pick(
		"Here's how I'd sequence it:\n\n- Get the local websocket answering `bootstrap` with the same shape this app already renders.\n- Then swap the mock store for those events, one screen at a time.\n- Pairing last, because it is the only part that needs two machines to test.\n\nWant me to hand the first piece to Developer?",
		"The constraint that decides this is **a bot runs on its assigned Runner**. Its files and plugins are there, so the answer is a job envelope, not a call from here.",
		"Short answer: yes. Longer answer: yes, but not until pairing works on two machines, because that is where this gets interesting.",
	)
}
