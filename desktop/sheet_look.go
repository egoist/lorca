package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// How a bot looks, after the macOS app's BotLookViewController: a symbol on an accent gradient, or an
// image of the user's own. The image wins while it is set; the symbol and accent stay underneath
// for when it is removed and for Devices that have not fetched it yet.

// lookSymbols are the symbols offered, the same list the phone app shows.
var lookSymbols = []string{
	"sparkles", "wand.and.stars", "hammer.fill", "book.fill",
	"paintbrush.fill", "chart.bar.fill", "terminal.fill", "globe",
	"brain.head.profile", "magnifyingglass", "envelope.fill", "calendar",
	"flask.fill", "bolt.fill", "leaf.fill", "shield.fill",
	"binoculars.fill", "chevron.left.forwardslash.chevron.right", "pencil.and.scribble", "bolt.horizontal.fill",
	"flame.fill",
}

// lookSymbolsPerRow is the symbol grid's columns.
const lookSymbolsPerRow = 8

// accentTitle is an accent's name, as its swatch's tooltip says it.
func accentTitle(accent model.Accent) string {
	switch accent {
	case "blue":
		return L("Blue")
	case "teal":
		return L("Teal")
	case "green":
		return L("Green")
	case "orange":
		return L("Orange")
	case "pink":
		return L("Pink")
	case "purple":
		return L("Purple")
	case "red":
		return L("Red")
	}
	return L("Indigo")
}

type lookImageChange int

const (
	lookImageKeep lookImageChange = iota
	lookImageRemove
	lookImageSet
)

type lookState struct {
	botID      string
	symbolName string
	accent     model.Accent
	change     lookImageChange
	// file is the prepared image while change is lookImageSet.
	file model.AvatarFile
}

// presentBotLook is a bot's look: its symbol, accent, and image.
func (w *appWindow) presentBotLook(botID string) {
	st := &lookState{botID: botID, symbolName: "sparkles", accent: "indigo"}
	if bot := store.Bot(botID); bot != nil {
		st.symbolName, st.accent = bot.SymbolName, bot.Accent
	}
	w.present(func(c *ui.Context, s *sheet) { st.view(c, s, w) }, nil)
}

// hasImage is whether the saved look, with the pending change applied, has an image.
func (st *lookState) hasImage() bool {
	switch st.change {
	case lookImageSet:
		return true
	case lookImageRemove:
		return false
	}
	bot := store.Bot(st.botID)
	return bot != nil && bot.Avatar != nil
}

// preview is the look as it will be saved.
func (st *lookState) preview() avatarContent {
	switch st.change {
	case lookImageSet:
		return avatarContent{Kind: avatarImage, Path: st.file.Path}
	case lookImageKeep:
		if bot := store.Bot(st.botID); bot != nil {
			if saved := botAvatar(bot); saved.Kind == avatarImage {
				return saved
			}
		}
	}
	return avatarContent{Kind: avatarBot, SymbolName: st.symbolName, Accent: st.accent}
}

// chooseImage asks for a picture and prepares it off the main thread; the sheet shows it once it
// is ready.
func (st *lookState) chooseImage(w *appWindow) {
	chooseFiles(w.win, "", L("Choose an image for this bot."), "", false, true, func(files []fileInfo) {
		if len(files) == 0 {
			return
		}
		path := files[0].Path
		go func() {
			file, err := prepareAvatar(path)
			post(func() {
				if err != nil {
					w.showAlert(alertOptions{Message: L("That file could not be read as an image.")}, nil)
					return
				}
				st.change, st.file = lookImageSet, file
			})
		}()
	})
}

func (st *lookState) save() {
	bot := store.Bot(st.botID)
	if bot == nil {
		return
	}
	if st.symbolName != bot.SymbolName || st.accent != bot.Accent {
		store.SetBotLook(bot.ID, st.symbolName, st.accent)
	}
	switch st.change {
	case lookImageRemove:
		store.SetBotAvatar(bot.ID, nil)
	case lookImageSet:
		file := st.file
		store.SetBotAvatar(bot.ID, &file)
	}
}

func (st *lookState) view(c *ui.Context, s *sheet, w *appWindow) {
	p := colors(c)
	result := sheetFrame(c, sheetOptions{
		Title:    L("Look"),
		Subtitle: L("Pick a symbol and a color, or use an image of your own. Paired Devices see the same look."),
		Width:    400,
		Confirm:  L("Save"),
	}, func() {
		ui.Column(c).Children(func() {
			ui.Row(c).Justify(ui.Center).Margin(0, 0, 18, 0).Children(func() {
				avatar(c, st.preview(), 72, false)
			})
			lookHeading(c, L("Symbol"))
			ui.Column(c).Gap(6).Margin(0, 0, 16, 0).Children(func() {
				for start := 0; start < len(lookSymbols); start += lookSymbolsPerRow {
					names := lookSymbols[start:min(start+lookSymbolsPerRow, len(lookSymbols))]
					ui.Row(c).Gap(6).Children(func() {
						for _, name := range names {
							selected := name == st.symbolName
							tile := ui.ButtonBase(c).Size(38, 34).Radius(8).Center().Background(p.Chip).TextColor(p.Label).
								Cursor(ui.CursorPointer).Tooltip(name).Label(name).Role(ui.RoleToggleButton).Checked(selected)
							if selected {
								tile.Background(p.accentColor(st.accent)).TextColor(p.White)
							}
							tile.Children(func() { symbol(c, name, 16, 2.2) })
							if tile.Clicked() {
								st.symbolName = name
							}
						}
					})
				}
			})
			lookHeading(c, L("Color"))
			ui.Row(c).Gap(8).Margin(0, 0, 16, 0).Children(func() {
				for _, accent := range model.Accents {
					title := accentTitle(accent)
					swatch := ui.ButtonBase(c).Size(26, 26).Radius(13).Background(p.accentColor(accent)).Cursor(ui.CursorPointer).
						Tooltip(title).Label(title).Role(ui.RoleToggleButton).Checked(accent == st.accent)
					if accent == st.accent {
						swatch.Border(2.5, p.Label)
					}
					if swatch.Clicked() {
						st.accent = accent
					}
				}
			})
			lookHeading(c, L("Image"))
			ui.Row(c).Gap(8).Margin(0, 0, 12, 0).Children(func() {
				if pushButton(c, L("Choose Image…"), pushOptions{}).Clicked() {
					st.chooseImage(w)
				}
				if st.hasImage() && pushButton(c, L("Remove Image"), pushOptions{}).Clicked() {
					st.change = lookImageRemove
				}
			})
			text := L("Images are resized to %d px and shared encrypted, like an attachment.", avatarSide)
			if st.hasImage() {
				text = L("The image shows in place of the symbol and color. It is resized to %d px and shared encrypted, like an attachment.", avatarSide)
			}
			ui.Text(c, text).FontSize(textCaption).LineHeight(1.4).TextColor(p.Label3)
		})
	})
	switch {
	case result.Confirmed:
		st.save()
		s.dismiss()
	case result.Cancelled:
		s.dismiss()
	}
}

// lookHeading is a small caption over a group of choices.
func lookHeading(c *ui.Context, title string) {
	ui.Text(c, strings.ToUpper(title)).FontSize(10).FontWeight(600).TextColor(colors(c).Label3).Margin(0, 0, 6, 0)
}
