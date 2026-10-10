package main

import (
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// email is the account's email address, which every bot shares, after the macOS app's
// EmailSettingsViewController: getting one, copying, changing, and giving it up. Listed while the
// relay offers email; the CLI asks the relay when the pane comes up.
func (s settingsPane) email(c *ui.Context) {
	p := colors(c)
	mail := store.Mail
	s.frame(c, string(model.PaneEmail), func() {
		if mail == nil || !mail.Available {
			// Only a page picked before the relay stopped offering email shows this.
			if mail != nil {
				s.footnote(c, L("Your relay doesn't offer email addresses."))
			}
			return
		}
		label := emailAddressEntry().row
		s.section(c, emailAddressEntry().title, nil, func(k *card) {
			address := mail.Address
			if address == nil {
				rowElement, row := actionRow(c, k, label, actionRowOptions{Tint: &p.Label2, Action: L("Get an Address…")})
				s.mark(c, rowElement, label)
				if row.Action {
					s.w.presentMailAddress()
				}
				return
			}
			copied := false
			if s.page != nil && !s.page.mailCopiedAt.IsZero() {
				if since := c.Now().Sub(s.page.mailCopiedAt); since < 1500*time.Millisecond {
					copied = true
					c.After(1500*time.Millisecond - since)
				}
			}
			copyTitle := L("Copy")
			if copied {
				copyTitle = L("Copied")
			}
			rowElement, row := actionRow(c, k, label, actionRowOptions{Value: address.Email, Tint: &p.Label, Action: copyTitle, Copied: copied, Second: L("Change…")})
			rowElement.ContextMenu(func(menu *ui.Menu) {
				if menu.Item(L("Copy Address")).Chosen() {
					copyText(address.Email)
				}
				if menu.Item(L("Change Address…")).Chosen() {
					s.w.presentMailAddress()
				}
				menu.Separator()
				if menu.Item(L("Give Up Address…")).Chosen() {
					s.w.confirmReleaseMail(*address)
				}
			})
			s.mark(c, rowElement, label)
			switch {
			case row.Action:
				copyText(address.Email)
				if s.page != nil {
					s.page.mailCopiedAt = c.Now()
				}
			case row.Second:
				s.w.presentMailAddress()
			}
			if mail.IsSuspended() {
				keyValueRow(c, k, L("Status"), L("Suspended"), false, &p.Red)
			}
			if _, giveUp := actionRow(c, k, L("Give Up"), actionRowOptions{Tint: &p.Label2, Action: L("Give Up Address…")}); giveUp.Action {
				s.w.confirmReleaseMail(*address)
			}
		})
		if mail.Address == nil {
			s.footnote(c, L("Your bots can use an address of their own to sign up for services, write to people for you, and schedule time. Lorca keeps the mail it receives encrypted for your Runners."))
			return
		}
		var notes []string
		if mail.IsSuspended() {
			notes = append(notes, L("Mail to this address bounces for now, because mail sent from it kept bouncing."))
		}
		if routing := mailRoutingNote(*mail); routing != "" {
			notes = append(notes, routing)
		}
		if len(notes) > 0 {
			s.footnote(c, strings.Join(notes, " "))
		}
	})
}

// mailRoutingNote says where mail goes: a bot's own address to that bot, the rest to the lead bot.
func mailRoutingNote(mail model.MailStatus) string {
	lead := store.Bot(mail.LeadBotID)
	if lead == nil {
		return ""
	}
	for _, bot := range store.Bots {
		if email := mail.BotEmail(bot.ID); bot.ID != lead.ID && email != "" {
			return L("Each bot writes from an address of its own, such as %@, and mail to it goes to that bot. Other mail goes to %@.", email, lead.Name)
		}
	}
	return L("Mail to this address goes to %@.", lead.Name)
}

// mailNameChoice is how the sheet names the address.
type mailNameChoice int

const (
	mailRandomName mailNameChoice = iota
	mailOwnName
)

// mailNameAllowed is whether every character is one the relay takes in a name. A name too short,
// or with a dot at an end, only keeps the button off.
func mailNameAllowed(name string) bool {
	return !strings.ContainsFunc(name, func(r rune) bool {
		return !(r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '.' || r == '-')
	})
}

// presentMailAddress gets the account an email address, or changes the one it has, after the
// macOS app's EmailAddressViewController: a random name, or one of the user's own when nobody has
// it. Changing gives the old name up.
func (w *appWindow) presentMailAddress() {
	mail := store.Mail
	if mail == nil || !mail.Available {
		return
	}
	current, domain := mail.Address, mail.Domain
	choice, name := mailRandomName, ""
	busy, problem, focusName := false, "", false
	w.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		title, confirm := L("Get an Email Address"), L("Get Address")
		if current != nil {
			title, confirm = L("Change Email Address"), L("Change Address")
		}
		normalized := model.NormalizeMailName(name)
		own := choice == mailOwnName
		// A name with a character the relay takes no part of says so at once.
		shown := problem
		if own && !mailNameAllowed(normalized) {
			shown = model.MailProblemText(model.MailInvalid)
		}
		result := sheetFrame(c, sheetOptions{
			Title:           title,
			Width:           420,
			Confirm:         confirm,
			ConfirmDisabled: busy || own && !model.ValidMailName(normalized),
		}, func() {
			ui.RadioGroup(c, func() {
				ui.Column(c).Gap(8).Children(func() {
					if ui.Radio(c, &choice, mailRandomName, L("Random name")).FontSize(13).Disabled(busy).Changed() {
						problem = ""
					}
					if ui.Radio(c, &choice, mailOwnName, L("Your own name")).FontSize(13).Disabled(busy).Changed() {
						focusName = true
					}
				})
			}).Label(L("Address"))
			// The field and what is wrong with it sit under their radio button's title.
			ui.Column(c).Gap(6).Margin(-4, 0, 0, 20).Children(func() {
				ui.Row(c).Gap(4).AlignItems(ui.Center).Children(func() {
					field := textField(c, &name, fieldOptions{Placeholder: L("name"), Label: L("Your own name"), Disabled: busy}).Grow(1).MinWidth(0)
					if focusName {
						focusName = false
						field.Focus()
					}
					if field.Changed() {
						name = strings.ToLower(name)
						problem = ""
						// Typing a name picks it.
						if name != "" {
							choice = mailOwnName
						}
					}
					ui.Text(c, "@"+domain).FontSize(13).TextColor(p.Label2).SingleLine()
				})
				if shown != "" {
					ui.Text(c, shown).FontSize(textCaption).TextColor(p.Red).LineHeight(1.4)
				}
			})
			if current != nil {
				caption(c, L("%@ stops working: mail to it bounces, and nobody else can take the name.", current.Email)).LineHeight(1.4).Margin(4, 0, 0, 0)
			}
		})
		switch {
		case result.Cancelled && !busy:
			s.dismiss()
		case result.Confirmed && !busy:
			wanted := ""
			if own {
				wanted = normalized
			}
			if current != nil && wanted == current.Name {
				s.dismiss()
				return
			}
			busy, problem = true, ""
			store.ApplyMail(wanted, func(refused string, err error) {
				busy = false
				switch {
				case err != nil:
					message := L("Couldn't get an address")
					if current != nil {
						message = L("Couldn't change the address")
					}
					w.showNote(message, model.ErrorText(err))
				case refused != "":
					problem, focusName = model.MailProblemText(refused), true
				default:
					s.dismiss()
				}
				w.invalidate()
			})
		}
	}, nil)
}

// confirmReleaseMail asks before giving the address up.
func (w *appWindow) confirmReleaseMail(address model.MailAddress) {
	w.showAlert(alertOptions{
		Message:     L("Give up %@?", address.Email),
		Informative: L("Mail to it will bounce, and nobody else can take the name."),
		Buttons:     []alertButton{{Title: L("Give Up"), Destructive: true}, {Title: L("Cancel")}},
	}, func(answer int) {
		if answer != 0 {
			return
		}
		store.ReleaseMail(func(err error) {
			if err != nil {
				w.showNote(L("Couldn't give up the address"), model.ErrorText(err))
			}
		})
	})
}
