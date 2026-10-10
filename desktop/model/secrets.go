package model

import "slices"

// Secrets the user saved for bots: a password, an API key, or a one-time code a bot asked for on a
// card. They are kept on the bot's Runner, and a value only ever travels there: when the user
// answers a card or replaces one. The Secrets pane lists what each is, never its value.

// Secrets answers the secrets kept on `runnerID`, by bot and then label.
func (s *Store) Secrets(runnerID string, done func([]SavedSecret, error)) {
	if s.IsMock {
		var secrets []SavedSecret
		for _, saved := range s.mockSecretList() {
			if saved.runnerID == runnerID {
				secrets = append(secrets, saved.secret)
			}
		}
		s.post(func() { done(secrets, nil) })
		return
	}
	Async(s, func() ([]SavedSecret, error) {
		var answer struct {
			Secrets []WireSecret `json:"secrets"`
		}
		err := s.request("secrets.list", map[string]any{"runner_id": runnerID}, &answer)
		secrets := make([]SavedSecret, 0, len(answer.Secrets))
		for _, wire := range answer.Secrets {
			secrets = append(secrets, wire.model())
		}
		return secrets, err
	}, done)
}

// ReplaceSecret gives a secret a new value, from the Secrets pane.
func (s *Store) ReplaceSecret(runnerID, id, value string, done func(error)) {
	if s.IsMock {
		s.post(func() { done(nil) })
		return
	}
	s.simple(done, "secrets.set", map[string]any{"runner_id": runnerID, "id": id, "value": value})
}

func (s *Store) DeleteSecret(runnerID, id string, done func(error)) {
	if s.IsMock {
		s.mockSecrets = slices.DeleteFunc(s.mockSecretList(), func(saved mockSecret) bool { return saved.secret.ID == id })
		s.post(func() { done(nil) })
		return
	}
	s.simple(done, "secrets.delete", map[string]any{"runner_id": runnerID, "id": id})
}

// AnswerSecret answers a secret request with a value for each of its fields, by name. The CLI
// seals them to the bot's Runner; the card reads Saved once the Runner has them. Fails with why
// they did not go, such as the Runner being offline.
func (s *Store) AnswerSecret(chatID, messageID string, values map[string]string, done func(error)) {
	saved := func(err error) {
		if err == nil {
			s.Update(messageID, chatID, func(message *Message) {
				if message.Body.Kind != BodyPermission || !message.Body.Request.IsPending() {
					return
				}
				request := *message.Body.Request
				request.Decision = DecisionAllowed
				message.Body.Request = &request
			})
		}
		done(err)
	}
	s.simple(saved, "chats.permission", map[string]any{"chat_id": chatID, "message_id": messageID, "decision": "allow", "values": values})
}

// mockSecret is a demo Runner's secret, with the Runner it is on.
type mockSecret struct {
	runnerID string
	secret   SavedSecret
}

func (s *Store) mockSecretList() []mockSecret {
	if s.mockSecrets == nil {
		s.mockSecrets = []mockSecret{
			{"dev-studio", SavedSecret{ID: "secret-github", BotID: "bot-patch", Name: "github_password", Label: "GitHub password", Use: SecretBrowser, Site: "github.com", UpdatedAt: float64(minutesAgo(60 * 26).Unix())}},
			{"dev-studio", SavedSecret{ID: "secret-s2", BotID: "bot-scout", Name: "S2_API_KEY", Label: "Semantic Scholar API key", Use: SecretCommand, UpdatedAt: float64(minutesAgo(60 * 24 * 6).Unix())}},
			{"dev-workbench", SavedSecret{ID: "secret-medium", BotID: "bot-quill", Name: "medium_password", Label: "Medium password", Use: SecretBrowser, Site: "medium.com", UpdatedAt: float64(minutesAgo(60 * 50).Unix())}},
		}
	}
	return s.mockSecrets
}
