package model

import "net/url"

// Output is additive encrypted message metadata. ID names the series; the enclosing message
// names one immutable version. TaskID references the canonical task and creates no task record.
type Output struct {
	ID                string          `json:"id"`
	Name              string          `json:"name"`
	Mime              string          `json:"mime"`
	BotID             string          `json:"bot_id"`
	ChatID            string          `json:"chat_id"`
	TaskID            string          `json:"task_id,omitempty"`
	Version           uint32          `json:"version"`
	PreviousMessageID string          `json:"previous_message_id,omitempty"`
	URL               string          `json:"url,omitempty"`
	Evidence          *OutputEvidence `json:"evidence,omitempty"`
}

type OutputEvidence struct {
	Kind     string `json:"kind"`
	Summary  string `json:"summary"`
	Status   string `json:"status"`
	Command  string `json:"command,omitempty"`
	ExitCode *int   `json:"exit_code,omitempty"`
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
		return L("Verification")
	}
}

func (e OutputEvidence) StatusText() string {
	switch e.Status {
	case "passed":
		return L("Passed")
	case "failed":
		return L("Failed")
	default:
		return L("Unverified")
	}
}

// DocumentURL accepts the same existing-document references as the publication API.
func (o Output) DocumentURL() string {
	parsed, err := url.Parse(o.URL)
	if err != nil || parsed.Scheme != "https" || parsed.Hostname() == "" || parsed.User != nil {
		return ""
	}
	return parsed.String()
}

type OutputPage struct {
	Messages []*Message
	HasMore  bool
}

type wireOutputPage struct {
	Outputs []WireMessage `json:"outputs"`
	HasMore bool          `json:"has_more"`
}

// LoadOutputs queries all locally synced versions, independently of the transcript's page.
// Its callback follows the store's ordered main-thread post queue.
func (s *Store) LoadOutputs(chatID, taskID string, done func(OutputPage, error)) {
	if s.IsMock {
		page := OutputPage{}
		chat := s.Chat(chatID)
		if chat == nil {
			done(page, &RequestError{L("Chat unavailable")})
			return
		}
		for _, message := range chat.Messages {
			if message.Output != nil && (taskID == "" || message.Output.TaskID == taskID) {
				page.Messages = append(page.Messages, message)
			}
		}
		page.HasMore = chat.HasMore
		done(page, nil)
		return
	}
	params := map[string]any{"chat_id": chatID}
	if taskID != "" {
		params["task_id"] = taskID
	}
	Async(s, func() (wireOutputPage, error) {
		return call[wireOutputPage](s, "outputs.list", params)
	}, func(reply wireOutputPage, err error) {
		page := OutputPage{HasMore: reply.HasMore}
		if err == nil {
			for _, wire := range reply.Outputs {
				if wire.Output != nil {
					page.Messages = append(page.Messages, ToMessage(wire))
				}
			}
		}
		done(page, err)
	})
}

func (s *Store) AttachmentError(id string) string { return s.attachmentErrors[id] }

// RetryAttachment performs an explicit re-fetch; failures stay visible until the next Retry.
func (s *Store) RetryAttachment(attachment Attachment, chatID, messageID string) {
	if s.fetchingAttachment[attachment.ID] {
		return
	}
	delete(s.attachmentErrors, attachment.ID)
	delete(s.attachmentFiles, attachment.ID)
	s.fetchAttachment(attachment, func() { s.emit(Event{Kind: EventMessageChanged, ChatID: chatID, MessageID: messageID}) })
	s.emit(Event{Kind: EventMessageChanged, ChatID: chatID, MessageID: messageID})
}
