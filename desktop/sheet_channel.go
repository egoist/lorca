package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// One channel's details, after the macOS app's ChannelViewController: whether it listens, and what
// holds it when a message's turn didn't finish or the account can't be read; where it listens and
// what it takes; the task the bot does with each message; and its conversations, each a click away.
// The bot sets a channel up and changes it when asked in chat; here the user pauses it, settles a
// held message, or removes it.

// presentChannel is a channel's details; openChat opens one of its conversations.
func (w *appWindow) presentChannel(channelID string, bot *model.Bot, openChat func(chatID string)) {
	if bot == nil {
		return
	}
	title := L("Channel")
	if channel := store.Channel(channelID); channel != nil {
		title = channel.Name
	}
	w.present(func(c *ui.Context, s *sheet) { w.channelView(c, s, title, channelID, bot, openChat) }, nil)
}

func (w *appWindow) channelView(c *ui.Context, s *sheet, title, channelID string, bot *model.Bot, openChat func(chatID string)) {
	p := colors(c)
	channel := store.Channel(channelID)
	// The sheet closes when the channel is gone.
	if channel == nil {
		s.dismiss()
	}
	result := sheetFrame(c, sheetOptions{
		Title:    title,
		Width:    480,
		Confirm:  L("Done"),
		NoCancel: true,
		Leading: func() {
			if channel != nil && pushButton(c, L("Remove…"), pushOptions{Kind: buttonDestructive}).Clicked() {
				w.confirmRemoveChannel(s, channel.ID, bot.Name)
			}
		},
	}, func() {
		if channel == nil {
			return
		}
		section(c, "", sectionCaption, nil, func(k *card) {
			on := !channel.IsPaused()
			id := channel.ID
			accessoryRow(c.Key("listening"), k, L("Listening"), "", func() {
				toggleSwitch(c, &on, true).Label(L("Listening")).OnChange(func() { store.SetChannelPaused(id, !on) })
			})
			switch {
			case channel.State == model.ChannelHeld && channel.HeldDelivery == "":
				// Nothing to settle: it waits for the user, or for its bot to come back.
				keyValueRow(c.Key("waiting"), k, L("State"), L("On hold"), false, &p.Orange)
				if channel.Detail != "" {
					noteRow(c.Key("waiting-note"), k, channel.Detail, nil)
				}
			case channel.State == model.ChannelHeld:
				_, held := actionRow(c.Key("held"), k, L("State"), actionRowOptions{Value: L("On hold"), Tint: &p.Orange, Action: L("Try Again"), Second: L("Skip")})
				if held.Action {
					store.SettleHeldMessage(id, true)
				}
				if held.Second {
					store.SettleHeldMessage(id, false)
				}
				noteRow(c.Key("held-note"), k, L("A message’s turn didn’t finish, so later messages wait. Read its conversation, then try it again or skip it."), nil)
			case channel.State == model.ChannelOffline:
				keyValueRow(c.Key("offline"), k, L("State"), L("Can’t connect"), false, &p.Orange)
				if channel.Detail != "" {
					noteRow(c.Key("offline-note"), k, channel.Detail, nil)
				}
			case channel.State == model.ChannelListening:
				if channel.Detail != "" {
					noteRow(c.Key("note"), k, channel.Detail, nil)
				}
			}
			chats := L("Every chat the bot is in")
			if len(channel.Chats) > 0 {
				var names []string
				for _, chat := range channel.Chats {
					name := chat.Title
					if name == "" {
						name = chat.ID
					}
					names = append(names, name)
				}
				chats = strings.Join(names, Lc(", ", "list"))
			}
			keyValueRow(c.Key("account"), k, L("Account"), store.AccountName(channel), false, nil)
			keyValueRow(c.Key("chats"), k, L("Chats"), chats, false, nil)
			keyValueRow(c.Key("messages"), k, L("Messages"), channel.Listen.Summary(), false, nil)
		})
		section(c, L("Task"), sectionCaption, nil, func(k *card) {
			k.row(ui.Row(c).Padding(10, 12).Children(func() {
				ui.Text(c, channel.Task).FontSize(12).LineHeight(1.4).Selectable()
			}))
		})
		kept := store.Conversations(channel.ID)
		if len(kept) > 6 {
			kept = kept[:6]
		}
		if len(kept) > 0 {
			section(c, L("Conversations"), sectionCaption, nil, func(k *card) {
				for _, chat := range kept {
					chatID := chat.ID
					if disclosureRow(c.Key(chat.ID), k, store.Title(chat), model.Stamp(chat.LastActivity()), nil) {
						s.dismiss()
						if openChat != nil {
							openChat(chatID)
						}
					}
				}
			})
		}
	})
	if result.Confirmed || result.Cancelled {
		s.dismiss()
	}
}

// confirmRemoveChannel asks before removing a channel; its conversations stay.
func (w *appWindow) confirmRemoveChannel(s *sheet, channelID, botName string) {
	channel := store.Channel(channelID)
	if channel == nil {
		return
	}
	w.showAlert(alertOptions{
		Message:     L("Remove “%@”?", channel.Name),
		Informative: L("%@ stops listening there. Its conversations stay.", botName),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Remove"), Destructive: true}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 {
			return
		}
		store.RemoveChannel(channelID)
		s.dismiss()
	})
}
