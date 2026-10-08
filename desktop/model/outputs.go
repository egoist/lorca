package model

import (
	"log"
	"net/url"
	"slices"
	"strings"
)

// Output is a file or document link a bot published in a chat: one version of it. ID names the
// output across its versions; the message that carries it is the version.
type Output struct {
	ID       string          `json:"id"`
	Name     string          `json:"name"`
	Mime     string          `json:"mime"`
	BotID    string          `json:"bot_id"`
	Version  uint32          `json:"version"`
	URL      string          `json:"url,omitempty"`
	Evidence *OutputEvidence `json:"evidence,omitempty"`
}

// OutputEvidence is what the bot says it checked: a test run, a screenshot from before or after a
// change, or another check, and how it went.
type OutputEvidence struct {
	Kind    string `json:"kind"`
	Summary string `json:"summary"`
	Status  string `json:"status"`
	Command string `json:"command,omitempty"`
}

func (e OutputEvidence) Title() string {
	switch e.Kind {
	case "test_result":
		return L("Test result")
	case "before_screenshot":
		return L("Before screenshot")
	case "after_screenshot":
		return L("After screenshot")
	default:
		return L("Check")
	}
}

func (e OutputEvidence) StatusText() string {
	switch e.Status {
	case "passed":
		return L("Passed")
	case "failed":
		return L("Failed")
	default:
		return L("Not verified")
	}
}

func (e OutputEvidence) Failed() bool { return e.Status == "failed" }

// DocumentURL is the document a link output points at; only an https address without credentials
// opens.
func (o Output) DocumentURL() string {
	parsed, err := url.Parse(o.URL)
	if o.URL == "" || err != nil || parsed.Scheme != "https" || parsed.Hostname() == "" || parsed.User != nil {
		return ""
	}
	return parsed.String()
}

// Symbol is the SF Symbol name for what the output is.
func (o Output) Symbol() string {
	switch {
	case o.URL != "":
		return "link"
	case strings.HasPrefix(o.Mime, "image/"):
		return "photo"
	case strings.HasPrefix(o.Mime, "video/"):
		return "film"
	case strings.HasPrefix(o.Mime, "audio/"):
		return "waveform"
	case o.Mime == "application/pdf":
		return "doc.richtext"
	case strings.HasPrefix(o.Mime, "text/") || o.Mime == "application/json":
		return "doc.text"
	}
	return "doc"
}

// OutputSeries is an output and its versions, newest first. Each version is a message of its own.
type OutputSeries struct {
	Versions []*Message
}

func (s OutputSeries) ID() string       { return s.Versions[0].Output.ID }
func (s OutputSeries) Latest() *Message { return s.Versions[0] }
func (s OutputSeries) Output() *Output  { return s.Versions[0].Output }

// GroupOutputs sorts a chat's output messages into one series per output, the latest published
// first.
func GroupOutputs(messages []*Message) []OutputSeries {
	var order []string
	byID := map[string][]*Message{}
	for _, message := range messages {
		if message.Output == nil {
			continue
		}
		id := message.Output.ID
		if _, ok := byID[id]; !ok {
			order = append(order, id)
		}
		byID[id] = append(byID[id], message)
	}
	series := make([]OutputSeries, 0, len(order))
	for _, id := range order {
		versions := slices.Clone(byID[id])
		slices.SortStableFunc(versions, func(a, b *Message) int { return int(b.Output.Version) - int(a.Output.Version) })
		series = append(series, OutputSeries{Versions: versions})
	}
	slices.SortStableFunc(series, func(a, b OutputSeries) int { return b.Latest().CreatedAt.Compare(a.Latest().CreatedAt) })
	return series
}

// Outputs are the chat's outputs, the latest published first: what `outputs.list` answered and
// output messages that arrived since. A chat is asked for when something first shows it, and again
// after a resync while what it had stays on screen; until the CLI answers there are none.
func (s *Store) Outputs(chatID string) []OutputSeries {
	if s.IsMock {
		if chat := s.Chat(chatID); chat != nil {
			return GroupOutputs(chat.Messages)
		}
		return nil
	}
	known, ok := s.outputMessages[chatID]
	if !ok || s.staleOutputs[chatID] {
		s.listOutputs(chatID)
	}
	return GroupOutputs(known)
}

func (s *Store) listOutputs(chatID string) {
	if s.outputRequests[chatID] {
		return
	}
	s.outputRequests[chatID] = true
	delete(s.staleOutputs, chatID)
	identity := s.IdentityID
	Async(s, func() ([]WireMessage, error) {
		reply, err := call[struct {
			Outputs []WireMessage `json:"outputs"`
		}](s, "outputs.list", map[string]any{"chat_id": chatID})
		return reply.Outputs, err
	}, func(wire []WireMessage, err error) {
		delete(s.outputRequests, chatID)
		if identity != s.IdentityID || s.Chat(chatID) == nil {
			return
		}
		if err != nil {
			log.Printf("listing outputs failed: %s", ErrorText(err))
			return
		}
		messages := make([]*Message, 0, len(wire))
		for _, each := range wire {
			messages = append(messages, ToMessage(each))
		}
		// Output messages that arrived while the list was on its way are in it too.
		for _, message := range s.outputMessages[chatID] {
			if !slices.ContainsFunc(messages, func(m *Message) bool { return m.ID == message.ID }) {
				messages = append(messages, message)
			}
		}
		s.outputMessages[chatID] = messages
		s.emit(Event{Kind: EventOutputsChanged, ChatID: chatID})
	})
}

// noteOutput keeps a chat's known outputs in step with a message that was added, changed, or (nil,
// with its id) removed.
func (s *Store) noteOutput(message *Message, removed, chatID string) {
	messages, ok := s.outputMessages[chatID]
	if !ok {
		return
	}
	id := removed
	if message != nil {
		id = message.ID
	}
	index := slices.IndexFunc(messages, func(m *Message) bool { return m.ID == id })
	if index < 0 && (message == nil || message.Output == nil) {
		return
	}
	next := slices.Clone(messages)
	if index >= 0 {
		next = slices.Delete(next, index, index+1)
	}
	if message != nil && message.Output != nil {
		next = append(next, message)
	}
	s.outputMessages[chatID] = next
	s.emit(Event{Kind: EventOutputsChanged, ChatID: chatID})
}

// AttachmentError is why the attachment's bytes could not be fetched, until a retry.
func (s *Store) AttachmentError(id string) string { return s.attachmentErrors[id] }

// RetryAttachment fetches a file whose fetch failed again.
func (s *Store) RetryAttachment(attachment Attachment, chatID, messageID string) {
	if _, failed := s.attachmentErrors[attachment.ID]; !failed {
		return
	}
	delete(s.attachmentErrors, attachment.ID)
	s.LocalFile(attachment, chatID, messageID)
	s.emit(Event{Kind: EventMessageChanged, ChatID: chatID, MessageID: messageID})
}

// OpenableFile is the attachment as a file named for what it is, for another app: the bytes under
// their attachment id carry no extension, so the CLI keeps a named copy. done runs on the main
// thread.
func (s *Store) OpenableFile(attachment Attachment, done func(string, error)) {
	if path := s.attachmentFiles[attachment.ID]; path != "" && strings.Contains(path[max(0, strings.LastIndexAny(path, `/\`)):], ".") {
		done(path, nil)
		return
	}
	Async(s, func() (string, error) {
		reply, err := call[struct {
			Path string `json:"path"`
		}](s, "files.path", map[string]any{"attachment": map[string]any{"id": attachment.ID, "name": attachment.Name, "mime": attachment.Mime, "size": attachment.Size}, "named": true})
		if err == nil && reply.Path == "" {
			err = &RequestError{L("File unavailable")}
		}
		return reply.Path, err
	}, done)
}
