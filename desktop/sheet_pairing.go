package main

import (
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
	"rsc.io/qr"
)

// Pairing another Device, after the macOS app's PairingSheetViewController: a pairing string for the
// other Device to scan or paste. The CLI runs the handshake and wraps the account key to the joining
// machine; this sheet only polls for the outcome. Done keeps the code good for ten minutes; Cancel
// retires it.

// pairingPollInterval is how often the sheet asks the CLI whether the other Device joined.
const pairingPollInterval = 1500 * time.Millisecond

type pairingState struct {
	pairingString string
	nonce         string
	// paired is the name of the Device that joined, once one did.
	paired string
	failed string
	// stopped is the sheet gone: answers that come back after it are dropped.
	stopped bool
}

// presentPairing pairs another Device.
func (w *appWindow) presentPairing() {
	st := &pairingState{}
	w.present(func(c *ui.Context, s *sheet) { st.view(c, s) }, func() { st.stopped = true })
	st.begin()
}

// begin asks the CLI for a pairing code, then polls for the other Device until it joins, the
// handshake fails, or the sheet goes.
func (st *pairingState) begin() {
	store.StartPairing(func(started model.WirePairStart, err error) {
		if st.stopped {
			return
		}
		if err != nil {
			st.failed = model.ErrorText(err)
			return
		}
		st.nonce, st.pairingString = started.Nonce, started.PairingString
		if !store.IsMock {
			st.poll()
		}
	})
}

func (st *pairingState) poll() {
	time.AfterFunc(pairingPollInterval, func() {
		post(func() {
			if st.stopped {
				return
			}
			store.PairingStatus(st.nonce, func(status model.WirePairStatus, err error) {
				if st.stopped {
					return
				}
				switch {
				case err != nil:
					st.failed = model.ErrorText(err)
				case status.State == "completed":
					// The code is spent: cover it, retire the copy button, and say who joined.
					st.paired = L("the other Device")
					if status.Device != nil && status.Device.Name != "" {
						st.paired = status.Device.Name
					}
				case status.State == "failed":
					st.failed = L("Pairing failed")
					if status.Error != nil && *status.Error != "" {
						st.failed = *status.Error
					}
				default:
					st.poll()
				}
			})
		})
	})
}

func (st *pairingState) view(c *ui.Context, s *sheet) {
	p := colors(c)
	result := sheetFrame(c, sheetOptions{
		Title:    L("Pair a Device"),
		Subtitle: L("On the other Device, choose Pair in onboarding (or run `lorca pair <code>`) and paste this code. The Devices run a handshake; the relay only carries ciphertext."),
		Width:    400,
		Confirm:  L("Done"),
	}, func() {
		ui.Column(c).Gap(12).AlignItems(ui.Center).Children(func() {
			frame := ui.Box(c).Padding(10).Radius(12).Background(p.White)
			frame.Children(func() {
				pairingQR(c, st.pairingString, 180)
				if st.paired != "" {
					ui.Row(c).Absolute().Left(0).Top(0).Right(0).Bottom(0).Center().Radius(12).
						Background(ui.RGBA(255, 255, 255, 0.9)).TextColor(lightPalette.Green).Label(L("Paired")).Children(func() {
						symbol(c, "checkmark.circle.fill", 64, 1.5)
					})
				}
			})
			ui.Row(c).AlignSelf(ui.Stretch).Gap(8).Padding(8, 8, 8, 10).Radius(8).Background(p.Code).Children(func() {
				if st.paired != "" {
					ui.Text(c, L("Paired with %@. This code is used up; pair another Device with a fresh one.", st.paired)).
						Grow(1).Shrink(1).MinWidth(0).FontSize(12).LineHeight(1.4)
					return
				}
				code := firstNonEmpty(st.pairingString, L("Asking the CLI for a pairing code…"))
				ui.Text(c, code).Grow(1).Shrink(1).MinWidth(0).Font(monoFont).FontSize(10).TextColor(p.Label2).MaxLines(3).Selectable()
				if st.pairingString != "" {
					copyButton(c, st.pairingString, copyOptions{Symbol: "doc.on.doc", Tooltip: L("Copy pairing string")})
				}
			})
			ui.Row(c).AlignSelf(ui.Stretch).Gap(8).Children(func() {
				status, tint := L("Waiting for the other Device… Done keeps this code good for ten minutes; Cancel retires it."), p.Label2
				switch {
				case st.failed != "":
					status, tint = st.failed, p.Red
				case st.paired != "":
					status, tint = L("Paired. The account key is wrapped to that machine."), p.Green
				default:
					spinner(c, 14)
				}
				ui.Text(c, status).Grow(1).Shrink(1).MinWidth(0).FontSize(12).LineHeight(1.4).TextColor(tint)
			})
		})
	})
	switch {
	case result.Confirmed:
		// Done keeps the code good: the CLI goes on waiting for ten minutes, so copying the code
		// and closing this sheet before pasting it on the phone is fine.
		st.stopped = true
		s.dismiss()
	case result.Cancelled:
		// Cancel retires the code: the CLI stops waiting and the relay drops the mailbox, so a
		// Device that pastes it afterwards is told at once.
		st.stopped = true
		if st.nonce != "" && st.paired == "" {
			store.CancelPairing(st.nonce)
		}
		s.dismiss()
	}
}

// pairingQRCodes are the codes drawn so far, by text: a pairing string stays up for the life of
// the sheet, and encoding it every frame would be wasted work.
var pairingQRCodes = map[string]*qr.Code{}

// pairingQR is a QR code of `text`, `size` across: crisp dark modules on the white of the frame
// around it, which is its quiet zone. With no text yet it is the empty square the code will fill.
func pairingQR(c *ui.Context, text string, size float32) ui.Element {
	box := ui.Box(c).Size(size, size)
	if text == "" {
		return box
	}
	code, ok := pairingQRCodes[text]
	if !ok {
		code, _ = qr.Encode(text, qr.M)
		pairingQRCodes[text] = code
	}
	if code == nil {
		return box
	}
	box.Role(ui.RoleImage).Label(text)
	box.Draw(func(painter *ui.Painter, r ui.Rect) {
		count := code.Size
		module := r.W / float32(count)
		black := ui.RGB(0, 0, 0)
		for y := range count {
			// One rectangle per run of dark modules along a row; edges snap to whole pixels, so
			// neighbors meet without a seam.
			for x := 0; x < count; {
				if !code.Black(x, y) {
					x++
					continue
				}
				start := x
				for x < count && code.Black(x, y) {
					x++
				}
				painter.Fill(ui.Rect{X: r.X + float32(start)*module, Y: r.Y + float32(y)*module, W: float32(x-start) * module, H: module}, black, 0)
			}
		}
	})
	return box
}
