package main

import (
	"fmt"
	"strings"

	"github.com/egoist/mygo/ui"
)

// The content area's states besides a chat and Settings, after the macOS app's
// StateViewControllers: loading while the CLI starts, the recovery page while it is not answering,
// and the placeholder when nothing is selected. And the Help menu's notes.

// statePage centers a column of a state in the content.
func statePage(c *ui.Context, fn func()) {
	ui.Column(c).Grow(1).Center().Padding(24).Children(func() {
		ui.Column(c).MaxWidth(420).AlignItems(ui.Center).Gap(10).Children(fn)
	})
}

// loadingState is the first window, ready while the CLI starts and loads the account.
func loadingState(c *ui.Context) {
	p := colors(c)
	statePage(c, func() {
		spinner(c, 32)
		ui.Text(c, L("Loading…")).FontSize(12).TextColor(p.Label2)
	})
}

// offlineState shows while the local CLI is not answering on 127.0.0.1. Retry Connection is the
// default button: Return presses it while nothing else has the keyboard.
func offlineState(c *ui.Context, m *mainWindow) {
	p := colors(c)
	command := cliCommand()
	if !m.hasSheet() && c.Shortcut(0, ui.KeyEnter) {
		store.Reconnect()
	}
	statePage(c, func() {
		ui.Row(c).TextColor(p.Label3).Children(func() { symbol(c, "bolt.horizontal.circle", 44, 1.4) })
		ui.Text(c, L("The Lorca CLI isn't answering")).FontSize(17).FontWeight(700).TextAlign(ui.Center)
		ui.Text(c, L("Your bots, keys and transcripts live in the CLI on this computer. The app starts it on its own; you can also run it from a terminal, and this window reconnects either way.")).
			FontSize(12.5).TextColor(p.Label2).TextAlign(ui.Center).LineHeight(1.4)
		ui.Row(c).Gap(6).Padding(4, 6, 4, 10).Radius(7).Background(p.Code).Children(func() {
			ui.Text(c, "$ "+command).Font(monoFont).FontSize(12).Selectable()
			copyButton(c, command, copyOptions{Symbol: "doc.on.doc", Tooltip: L("Copy command")})
		})
		if pushButton(c, L("Retry Connection"), pushOptions{Kind: buttonPrimary, Large: true}).Margin(6, 0, 0, 0).Clicked() {
			store.Reconnect()
		}
		ui.Text(c, fmt.Sprintf("%s · 127.0.0.1:%d", store.OfflineStatus(), prefs.cliPort())).FontSize(textCaption).TextColor(p.Label3).TextAlign(ui.Center).Selectable()
	})
}

// placeholderState shows when nothing is selected in the sidebar.
func placeholderState(c *ui.Context, m *mainWindow) {
	p := colors(c)
	statePage(c, func() {
		ui.Row(c).TextColor(p.Label3).Children(func() { symbol(c, "bubble.left.and.bubble.right", 38, 1.4) })
		ui.Text(c, L("No chat selected")).FontSize(15).FontWeight(600)
		ui.Text(c, L("Pick a conversation in the sidebar, or start a new one.")).FontSize(12.5).TextColor(p.Label2).TextAlign(ui.Center)
		if pushButton(c, L("New Bot…"), pushOptions{Large: true}).Margin(6, 0, 0, 0).Clicked() {
			m.newBot()
		}
	})
}

// MARK: - Help

func showHelp(w *appWindow) {
	if w == nil {
		return
	}
	body := L("Every bot is assigned to a Runner: a Device running macOS, Linux, or Windows. That machine's CLI runs the turn with your account's provider credentials, so a bot on an offline Runner waits until it reconnects. Phones and tablets pair as Devices but never run bots.\n\nThe app talks only to the local CLI on 127.0.0.1:%@. Start it with `lorca serve`; the CLI holds your keys and provider credentials, which reach your other Devices encrypted.", fmt.Sprint(prefs.cliPort()))
	w.showNote(L("Lorca runs on Devices you own"), strings.Replace(body, "lorca serve", cliCommand(), 1))
}

func showArchitecture(w *appWindow) {
	if w == nil {
		return
	}
	w.showNote(L("Three processes"), L("The app talks only to the local CLI over a localhost websocket. The CLI holds the keys, runs the agent loop, and syncs ciphertext with the relay. The relay stores public keys and opaque blobs.\n\nFull notes live in ARCHITECTURE.md."))
}

func showAbout(w *appWindow) {
	if w == nil {
		return
	}
	w.showAlert(alertOptions{Message: appName(), Informative: L("Version %@", appVersion()), Escape: -1}, nil)
}
