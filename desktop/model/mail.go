package model

import (
	"crypto/rand"
	"os"
	"slices"
	"strings"
)

// The account's email address, after the macOS app's mail model: the relay hands it out on
// demand, every bot writes from its own tagged form of it, and the CLI asks the relay afresh on
// `mail.get`.

// MailAddress is the account's address while it has one.
type MailAddress struct {
	Name  string `json:"name"`
	Email string `json:"email"`
	// State is "active", or "suspended" while mail to it bounces because mail from it kept
	// bouncing.
	State string `json:"state"`
}

// MailBotAddress is the address a bot writes from: the account's with the bot's tag.
type MailBotAddress struct {
	BotID string `json:"bot_id"`
	Email string `json:"email"`
}

// MailStatus is what the relay says about email for the account.
type MailStatus struct {
	// Available is the relay offering email; without it the apps show nothing of it.
	Available bool         `json:"available"`
	Domain    string       `json:"domain"`
	Address   *MailAddress `json:"address"`
	// LeadBotID gets the mail that names no bot.
	LeadBotID string           `json:"lead_bot_id"`
	Bots      []MailBotAddress `json:"bots"`
}

// MailConnectionID is the built-in connection a bot's Access lists for email.
const MailConnectionID = "email"

// IsSuspended is mail to the address bouncing for now.
func (m MailStatus) IsSuspended() bool { return m.Address != nil && m.Address.State == "suspended" }

// BotEmail is the address the bot writes from, or "".
func (m MailStatus) BotEmail(botID string) string {
	for _, bot := range m.Bots {
		if bot.BotID == botID {
			return bot.Email
		}
	}
	return ""
}

// Mail problems the CLI answers `mail.apply` with instead of an address.
const (
	MailTaken    = "taken"
	MailReserved = "reserved"
	MailInvalid  = "invalid"
)

// MailProblemText is what the sheet says under the name for a problem.
func MailProblemText(problem string) string {
	switch problem {
	case MailTaken:
		return L("That name is taken.")
	case MailReserved:
		return L("That name is reserved.")
	case MailInvalid:
		return L("Use letters, digits, dots, and hyphens.")
	}
	return ""
}

// NormalizeMailName is a typed name as the relay takes it: trimmed and lowercased.
func NormalizeMailName(name string) string { return strings.ToLower(strings.TrimSpace(name)) }

// ValidMailName is the relay's rule for a name of the user's own, checked here first: 3 to 32
// letters, digits, dots, and hyphens, starting and ending with a letter or digit, no "..".
func ValidMailName(name string) bool {
	if len(name) < 3 || len(name) > 32 || strings.Contains(name, "..") {
		return false
	}
	alnum := func(r byte) bool { return r >= 'a' && r <= 'z' || r >= '0' && r <= '9' }
	for i := 0; i < len(name); i++ {
		if !alnum(name[i]) && name[i] != '.' && name[i] != '-' {
			return false
		}
	}
	return alnum(name[0]) && alnum(name[len(name)-1])
}

// MARK: - Store

// RefreshMail asks the CLI what the relay says now.
func (s *Store) RefreshMail() {
	if s.IsMock {
		return
	}
	Async(s, func() (MailStatus, error) { return call[MailStatus](s, "mail.get", nil) }, func(status MailStatus, err error) {
		if err == nil {
			s.setMail(status)
		}
	})
}

// ApplyMail gets the account an address, or changes the one it has: a random name when `name` is
// empty. `problem` is why the relay refused the name, which the sheet shows under it.
func (s *Store) ApplyMail(name string, done func(problem string, err error)) {
	name = NormalizeMailName(name)
	if name != "" && !ValidMailName(name) {
		done(MailInvalid, nil)
		return
	}
	if s.IsMock {
		problem := s.mockApplyMail(name)
		s.post(func() { done(problem, nil) })
		return
	}
	params := map[string]any{}
	if name != "" {
		params["name"] = name
	}
	type answer struct {
		Mail    *MailStatus `json:"mail"`
		Problem string      `json:"problem"`
	}
	Async(s, func() (answer, error) { return call[answer](s, "mail.apply", params) }, func(result answer, err error) {
		if err == nil && result.Mail != nil {
			s.setMail(*result.Mail)
		}
		done(result.Problem, err)
	})
}

// ReleaseMail gives the address up: mail to it bounces, and the name goes to nobody else.
func (s *Store) ReleaseMail(done func(error)) {
	if s.IsMock {
		if s.Mail != nil {
			status := *s.Mail
			status.Address, status.Bots = nil, nil
			s.setMail(status)
		}
		s.post(func() { done(nil) })
		return
	}
	type answer struct {
		Mail *MailStatus `json:"mail"`
	}
	Async(s, func() (answer, error) { return call[answer](s, "mail.release", nil) }, func(result answer, err error) {
		if err == nil && result.Mail != nil {
			s.setMail(*result.Mail)
		}
		done(err)
	})
}

func (s *Store) setMail(status MailStatus) {
	s.Mail = &status
	s.emit(Event{Kind: EventMailChanged})
}

// MARK: - Demo

// mockMailDomain is the production relay's mail domain, which the demo shows.
const mockMailDomain = "bots.lorca.app"

// mockMail is the demo account's address. `LORCA_MOCK_MAIL` shows the other states: `none` (no
// address yet), `suspended`, or `off` (a relay without email).
func mockMail(bots []*Bot) *MailStatus {
	state := os.Getenv("LORCA_MOCK_MAIL")
	if state == "off" {
		return &MailStatus{}
	}
	status := MailStatus{Available: true, Domain: mockMailDomain}
	if state == "none" {
		if len(bots) > 0 {
			status.LeadBotID = bots[0].ID
		}
		return &status
	}
	mockMailAddress(&status, "k7f3m9q2", bots)
	if state == "suspended" {
		status.Address.State = "suspended"
	}
	return &status
}

// mockMailAddress gives the demo account `name`, with each bot's tagged address.
func mockMailAddress(status *MailStatus, name string, bots []*Bot) {
	status.Address = &MailAddress{Name: name, Email: name + "@" + status.Domain, State: "active"}
	status.Bots = nil
	for _, bot := range bots {
		status.Bots = append(status.Bots, MailBotAddress{BotID: bot.ID, Email: name + "+" + mockMailTag(bot) + "@" + status.Domain})
	}
	if len(bots) > 0 {
		status.LeadBotID = bots[0].ID
	}
}

// mockMailTag is a bot's name as a tag, as the CLI makes one: lowercase letters and digits, the
// rest hyphens.
func mockMailTag(bot *Bot) string {
	var tag strings.Builder
	for _, r := range strings.ToLower(bot.Name) {
		switch {
		case r >= 'a' && r <= 'z' || r >= '0' && r <= '9':
			tag.WriteRune(r)
		case tag.Len() > 0 && !strings.HasSuffix(tag.String(), "-"):
			tag.WriteByte('-')
		}
	}
	if out := strings.TrimSuffix(tag.String(), "-"); out != "" {
		return out
	}
	return strings.TrimPrefix(bot.ID, "bot-")
}

// mockTakenMailNames stand in for names other accounts hold, and the relay's reserved ones.
var mockTakenMailNames = []string{"egoist", "hello", "team"}
var mockReservedMailNames = []string{"abuse", "admin", "postmaster", "support"}

func (s *Store) mockApplyMail(name string) string {
	switch {
	case slices.Contains(mockReservedMailNames, name):
		return MailReserved
	case slices.Contains(mockTakenMailNames, name):
		return MailTaken
	}
	if name == "" {
		const letters = "abcdefghijklmnopqrstuvwxyz0123456789"
		bytes := make([]byte, 8)
		_, _ = rand.Read(bytes)
		for i := range bytes {
			bytes[i] = letters[int(bytes[i])%len(letters)]
		}
		name = string(bytes)
	}
	status := MailStatus{Available: true, Domain: mockMailDomain}
	if s.Mail != nil {
		status = *s.Mail
	}
	mockMailAddress(&status, name, s.Bots)
	s.setMail(status)
	return ""
}
