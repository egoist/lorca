package main

import (
	"io"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// Outputs, after the macOS app: what a chat's bots published shows in the inspector, the latest
// three with View all for the rest. A row opens the output's sheet: a preview of the file or the
// link, what the bot checked, and its versions. Open hands the file to its app or the link to the
// browser; Save As… copies the file. The files come from the local CLI.

// outputSaveDestination asks where to save a copy; tests put their own in its place.
var outputSaveDestination = func(parent *mygo.Window, name string) (string, error) {
	return mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: parent, Title: L("Save As…"), ButtonLabel: L("Save"), DefaultPath: filepath.Base(name)})
}

// outputOpenPath and outputOpenURL hand a file or a link to the system; tests replace them.
var outputOpenPath = openFile
var outputOpenURL = func(link string) { _ = openExternal(link) }

// inspectorOutputs is the inspector's Outputs section, hidden while the chat has none.
func (m *mainWindow) inspectorOutputs(c *ui.Context, chat *model.Chat) {
	all := store.Outputs(chat.ID)
	if len(all) == 0 {
		return
	}
	shown := all[:min(3, len(all))]
	var accessory func()
	if len(all) > len(shown) {
		accessory = func() {
			// On the rows' trailing text edge, as a row's state is.
			b := ui.ButtonBase(c.Key("outputs-all")).Margin(0, 12, 0, 0).Label(L("View all")).Cursor(ui.CursorPointer)
			b.Children(func() { ui.Text(c, L("View all")).FontSize(11).TextColor(colors(c).Label2) })
			if b.Clicked() {
				m.presentAllOutputs(chat.ID)
			}
		}
	}
	section(c, L("Outputs"), sectionCaption, accessory, func(k *card) {
		for _, series := range shown {
			m.outputRow(c, k, chat, series)
		}
	})
}

// outputRow is an output's row: what it is, its name, when it was published (and by whom, in a
// group), and how its check went. A click opens the output; its menu has the file actions.
func (w *appWindow) outputRow(c *ui.Context, k *card, chat *model.Chat, series model.OutputSeries) {
	p := colors(c)
	output := series.Output()
	subtitle := model.Stamp(series.Latest().CreatedAt)
	if chat.IsGroup() {
		if bot := store.Bot(output.BotID); bot != nil {
			subtitle = bot.Name + " · " + subtitle
		}
	}
	o := statusRowOptions{Symbol: output.Symbol(), Title: output.Name, Subtitle: subtitle, Clickable: true, Tooltip: output.Name}
	if output.Evidence != nil {
		o.State = output.Evidence.StatusText()
		if output.Evidence.Failed() {
			o.StateColor = &p.Red
		}
	}
	var row statusRowResult
	var element ui.Element
	ui.Box(c.Key("output:" + series.ID())).Children(func() {
		element, row = statusRow(c, k, o)
	})
	latest := series.Latest()
	element.ContextMenu(func(menu *ui.Menu) { w.outputMenu(menu, latest) })
	if row.Clicked {
		w.presentOutput(chat.ID, series.ID())
	}
}

// outputMenu is Open and Save As…, or for a link Open Link and Copy Link.
func (w *appWindow) outputMenu(menu *ui.Menu, message *model.Message) {
	if link := message.Output.DocumentURL(); link != "" {
		if menu.Item(L("Open Link")).Chosen() {
			outputOpenURL(link)
		}
		if menu.Item(L("Copy Link")).Chosen() {
			mygo.Clipboard.WriteText(link)
		}
		return
	}
	if len(message.Attachments) == 0 {
		return
	}
	attachment := message.Attachments[0]
	if menu.Item(L("Open")).Chosen() {
		w.openAttachment(attachment)
	}
	if menu.Item(L("Save As…")).Chosen() {
		w.saveAttachment(attachment)
	}
}

// openOutput opens the version's file in its app, or its link in the browser.
func (w *appWindow) openOutput(message *model.Message) {
	if link := message.Output.DocumentURL(); link != "" {
		outputOpenURL(link)
	} else if len(message.Attachments) > 0 {
		w.openAttachment(message.Attachments[0])
	}
}

// openAttachment opens a file in its app, under its own name so the system knows what it is.
func (w *appWindow) openAttachment(attachment model.Attachment) {
	store.OpenableFile(attachment, func(path string, err error) {
		if err != nil {
			w.fileFailed(err)
			return
		}
		outputOpenPath(path)
	})
}

// saveAttachment asks where to put a copy of the file and writes it there.
func (w *appWindow) saveAttachment(attachment model.Attachment) {
	store.OpenableFile(attachment, func(source string, err error) {
		if err != nil {
			w.fileFailed(err)
			return
		}
		parent := w.win
		model.Async(store, func() (struct{}, error) {
			target, err := outputSaveDestination(parent, attachment.Name)
			if err != nil || target == "" {
				return struct{}{}, err
			}
			return struct{}{}, copyFile(source, target)
		}, func(_ struct{}, err error) {
			if err != nil {
				w.fileFailed(err)
			}
		})
	})
}

func (w *appWindow) fileFailed(err error) {
	w.showAlert(alertOptions{Message: L("Couldn't get this file"), Informative: model.ErrorText(err), Escape: -1}, func(int) {})
}

// copyFile writes a copy of source at target through a temporary file beside it, so a failed copy
// leaves what was there.
func copyFile(source, target string) error {
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer input.Close()
	if info, err := os.Stat(target); err == nil {
		if sourceInfo, err := input.Stat(); err == nil && os.SameFile(sourceInfo, info) {
			return nil
		}
	}
	temporary, err := os.CreateTemp(filepath.Dir(target), ".lorca-copy-*")
	if err != nil {
		return err
	}
	defer os.Remove(temporary.Name())
	_, err = io.Copy(temporary, input)
	if closeErr := temporary.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(temporary.Name(), target)
}

// outputPreview is what a file's preview shows once read: an image, or the start of a text file.
type outputPreview struct {
	bitmap  *ui.Bitmap
	text    string
	loading bool
}

// outputSheet is one output's sheet; picked is the version on screen, empty for the latest. shown
// is the output as last listed, kept while a resync lists the chat again.
type outputSheet struct {
	chatID, outputID string
	picked           string
	previews         map[string]*outputPreview
	shown            model.OutputSeries
}

// presentOutput puts up one output's sheet.
func (w *appWindow) presentOutput(chatID, outputID string) {
	state := &outputSheet{chatID: chatID, outputID: outputID, previews: map[string]*outputPreview{}}
	w.present(func(c *ui.Context, s *sheet) { state.view(c, s) }, nil)
}

func (o *outputSheet) series() (model.OutputSeries, bool) {
	for _, series := range store.Outputs(o.chatID) {
		if series.ID() == o.outputID {
			o.shown = series
		}
	}
	return o.shown, len(o.shown.Versions) > 0
}

func (o *outputSheet) view(c *ui.Context, s *sheet) {
	p := colors(c)
	series, ok := o.series()
	if !ok || store.Chat(o.chatID) == nil {
		s.dismiss()
		return
	}
	message := series.Latest()
	for _, version := range series.Versions {
		if version.ID == o.picked {
			message = version
		}
	}
	output := message.Output
	bot := L("A bot")
	if b := store.Bot(output.BotID); b != nil {
		bot = b.Name
	}
	var attachment *model.Attachment
	path, fileError := "", ""
	if len(message.Attachments) > 0 {
		attachment = &message.Attachments[0]
		path = store.LocalFile(*attachment, o.chatID, message.ID)
		fileError = store.AttachmentError(attachment.ID)
	}
	link := output.DocumentURL()
	confirm := L("Open")
	if fileError != "" {
		confirm = L("Try Again")
	}
	options := sheetOptions{Title: output.Name, Subtitle: bot + " · " + model.DaySeparator(message.CreatedAt), Width: 480, Confirm: confirm, Cancel: L("Done")}
	if attachment != nil {
		options.Leading = func() {
			if pushButton(c.Key("output-save"), L("Save As…"), pushOptions{Disabled: path == ""}).Clicked() {
				s.window.saveAttachment(*attachment)
			}
		}
	}
	result := sheetFrame(c, options, func() {
		if attachment != nil {
			o.previewBox(c, *attachment, message.ID, path, fileError)
		}
		if link != "" {
			section(c, L("Link"), sectionCaption, nil, func(k *card) {
				host := link
				if parsed := strings.TrimPrefix(link, "https://"); parsed != "" {
					host, _, _ = strings.Cut(parsed, "/")
				}
				if _, row := statusRow(c, k, statusRowOptions{Symbol: "link", Title: host, Subtitle: link, Clickable: true, Tooltip: link}); row.Clicked {
					outputOpenURL(link)
				}
			})
		}
		if check := output.Evidence; check != nil {
			section(c, check.Title(), sectionCaption, nil, func(k *card) {
				var tint *ui.Color
				if check.Failed() {
					tint = &p.Red
				}
				keyValueRow(c, k, L("Result"), check.StatusText(), false, tint)
				k.row(rowBox(c).Label(check.Summary)).Children(func() {
					ui.Text(c, check.Summary).FontSize(textCaption + 1).Grow(1).Shrink(1).MinWidth(0).Selectable()
				})
				if check.Command != "" {
					keyValueRow(c, k, L("Command"), check.Command, true, nil)
				}
			})
		}
		if len(series.Versions) > 1 {
			section(c, L("Versions"), sectionCaption, nil, func(k *card) {
				for _, version := range series.Versions {
					subtitle := model.DaySeparator(version.CreatedAt)
					if version.Output.Evidence != nil {
						subtitle += " · " + version.Output.Evidence.StatusText()
					}
					mark := ""
					if version.ID == message.ID {
						mark = "checkmark"
					}
					var row statusRowResult
					ui.Box(c.Key("version:" + version.ID)).Children(func() {
						_, row = statusRow(c, k, statusRowOptions{Symbol: mark, Title: L("Version %d", int(version.Output.Version)), Subtitle: subtitle, Clickable: true})
					})
					if row.Clicked {
						o.picked = version.ID
						if version.ID == series.Latest().ID {
							o.picked = ""
						}
					}
				}
			})
		}
	})
	switch {
	case result.Cancelled:
		s.dismiss()
	case result.Confirmed && fileError != "":
		store.RetryAttachment(*attachment, o.chatID, message.ID)
	case result.Confirmed:
		s.window.openOutput(message)
		s.dismiss()
	}
}

// previewBox shows the file: an image, the start of a text file, or what it is, as Quick Look does
// in the macOS app; while it is on its way, or when it could not be fetched, it says so.
func (o *outputSheet) previewBox(c *ui.Context, attachment model.Attachment, messageID, path, fileError string) {
	p := colors(c)
	box := ui.Column(c.Key("preview")).Height(260).Radius(10).Clip().Background(p.BotBubble).Border(1, p.BotBubbleBorder).
		AlignItems(ui.Center).Justify(ui.Center)
	box.Children(func() {
		placeholder := func(text, detail string) {
			ui.Column(c).Gap(4).Padding(0, 24).AlignItems(ui.Center).Children(func() {
				ui.Text(c, text).FontSize(13).TextColor(p.Label2).TextAlign(ui.Center)
				if detail != "" {
					ui.Text(c, detail).FontSize(11).TextColor(p.Label3).TextAlign(ui.Center)
				}
			})
		}
		switch {
		case fileError != "":
			placeholder(L("This file couldn't be downloaded."), strings.ToUpper(fileError[:1])+fileError[1:])
			return
		case path == "":
			placeholder(L("Downloading…"), "")
			return
		}
		preview := o.preview(attachment, messageID, path)
		switch {
		case preview.bitmap != nil:
			ui.Image(c, preview.bitmap).Width(440).MaxWidthPercent(100).Height(240).Fit(ui.Contain)
		case preview.text != "":
			ui.Scroll(c).FillWidth().Grow(1).MinHeight(0).Background(p.Code).Padding(8, 10).Children(func() {
				ui.Text(c, preview.text).Font(monoFont).FontSize(11.5).LineHeight(1.45).Selectable()
			})
		case !preview.loading:
			ui.Column(c).Gap(6).AlignItems(ui.Center).TextColor(p.Label2).Children(func() {
				symbol(c, "doc", 40, 1.2)
				ui.Text(c, attachment.Name).FontSize(13).FontWeight(500).TextColor(p.Label)
				ui.Text(c, model.SizeText(attachment.Size)).FontSize(11).TextColor(p.Label2)
			})
		}
	})
}

// preview reads the version's file once, off the main thread: an image, or up to 64 KB of a text
// file.
func (o *outputSheet) preview(attachment model.Attachment, messageID, path string) *outputPreview {
	if preview := o.previews[messageID]; preview != nil {
		return preview
	}
	preview := &outputPreview{loading: true}
	o.previews[messageID] = preview
	model.Async(store, func() (outputPreview, error) { return readPreview(attachment, path) }, func(read outputPreview, err error) {
		*preview = read
		invalidateWindows()
	})
	return preview
}

func readPreview(attachment model.Attachment, path string) (outputPreview, error) {
	var result outputPreview
	file, err := os.Open(path)
	if err != nil {
		return result, err
	}
	defer file.Close()
	switch {
	case attachment.IsImage():
		data, err := io.ReadAll(io.LimitReader(file, 100<<20))
		if err != nil {
			return result, err
		}
		result.bitmap, err = ui.DecodeBitmap(data)
		return result, err
	case strings.HasPrefix(attachment.Mime, "text/") || attachment.Mime == "application/json" || attachment.Mime == "application/xml":
		data, err := io.ReadAll(io.LimitReader(file, 64<<10))
		if err != nil {
			return result, err
		}
		// The cut may split the last character; a file that is not UTF-8 shows as a file.
		for cut := 0; cut < utf8.UTFMax && len(data) > 0 && !utf8.Valid(data); cut++ {
			data = data[:len(data)-1]
		}
		if utf8.Valid(data) {
			result.text = string(data)
		}
	}
	return result, nil
}

// presentAllOutputs lists every output of the chat, for the inspector's View all.
func (m *mainWindow) presentAllOutputs(chatID string) {
	m.present(func(c *ui.Context, s *sheet) {
		chat := store.Chat(chatID)
		if chat == nil {
			s.dismiss()
			return
		}
		result := sheetFrame(c, sheetOptions{Title: L("Outputs"), Width: 440, Confirm: L("Done"), NoCancel: true}, func() {
			ui.Scroll(c).MaxHeight(440).Children(func() {
				section(c, "", sectionCaption, nil, func(k *card) {
					for _, series := range store.Outputs(chatID) {
						s.window.outputRow(c, k, chat, series)
					}
				})
			})
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, nil)
}
