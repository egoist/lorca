package main

import (
	"image"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// sheetTester is the demo's main window with a sheet put up by `open`, drawn and settled. The
// inspector is closed, so the sheet's labels are the only ones of their kind the tester finds.
func sheetTester(t *testing.T, open func(m *mainWindow)) (*mainWindow, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = false
	tt := ui.NewTester(m.frame(m.view), 1000, 760)
	runPosts()
	tt.Frame()
	open(m)
	settle(tt)
	return m, tt
}

func demoMemory(t *testing.T, botID string) model.BotMemory {
	t.Helper()
	var memory model.BotMemory
	store.BotMemory(botID, func(m model.BotMemory, err error) {
		if err != nil {
			t.Fatal(err)
		}
		memory = m
	})
	runPosts()
	return memory
}

func TestRenderBotSheets(t *testing.T) {
	sheets := []struct {
		name string
		open func(m *mainWindow)
	}{
		{"sheet-newbot", func(m *mainWindow) { m.presentNewBot(nil) }},
		{"sheet-look", func(m *mainWindow) { m.presentBotLook("bot-nova") }},
		{"sheet-pairing", func(m *mainWindow) { m.presentPairing() }},
		{"sheet-routine", func(m *mainWindow) { m.presentRoutine("rt-reviews", store.Bot("bot-nova"), nil) }},
		{"sheet-routine-plain", func(m *mainWindow) { m.presentRoutine("rt-checklist", store.Bot("bot-nova"), nil) }},
		{"sheet-memory", func(m *mainWindow) { m.presentMemory(store.Bot("bot-nova"), demoMemory(t, "bot-nova"), nil) }},
		{"sheet-newgroup", func(m *mainWindow) { m.presentNewGroupChat(nil) }},
		{"sheet-botpicker", func(m *mainWindow) {
			m.presentBotPicker(L("Add a bot to %@", "Launch copy"), []*model.Bot{store.Bot("bot-patch"), store.Bot("bot-ember")}, nil)
		}},
		{"sheet-botdescription", func(m *mainWindow) { m.presentBotDescription("bot-nova") }},
		{"sheet-groupdescription", func(m *mainWindow) { m.presentGroupDescription("chat-relay") }},
		{"sheet-pairing-paired", func(m *mainWindow) {
			st := &pairingState{pairingString: "lorca://pair?n=1", nonce: "1", paired: "Studio"}
			m.present(func(c *ui.Context, s *sheet) { st.view(c, s) }, nil)
		}},
		{"sheet-pairing-failed", func(m *mainWindow) {
			st := &pairingState{pairingString: "lorca://pair?n=1", nonce: "1", failed: "The pairing code expired."}
			m.present(func(c *ui.Context, s *sheet) { st.view(c, s) }, nil)
		}},
		{"sheet-look-image", func(m *mainWindow) {
			file, err := prepareAvatar(lookTestImage(t))
			if err != nil {
				t.Fatal(err)
			}
			st := &lookState{botID: "bot-nova", symbolName: "globe", accent: "teal", change: lookImageSet, file: file}
			m.present(func(c *ui.Context, s *sheet) { st.view(c, s, &m.appWindow) }, nil)
			// The app reads images off the main thread and lands them on it, which a test has not.
			data, _ := os.ReadFile(file.Path)
			bitmap, _ := ui.DecodeBitmap(data)
			bitmaps.Lock()
			bitmaps.entries[file.Path] = &bitmapEntry{bitmap: bitmap}
			bitmaps.Unlock()
		}},
	}
	for _, each := range sheets {
		t.Run(each.name, func(t *testing.T) {
			m, tt := sheetTester(t, each.open)
			if !m.hasSheet() {
				t.Fatal("no sheet")
			}
			renderBoth(t, tt, each.name)
		})
	}
}

// lookTestImage is a wide picture with a gradient, for a profile image to crop.
func lookTestImage(t *testing.T) string {
	t.Helper()
	picture := image.NewNRGBA(image.Rect(0, 0, 600, 400))
	for y := range 400 {
		for x := range 600 {
			picture.SetNRGBA(x, y, color.NRGBA{R: uint8(x * 255 / 600), G: uint8(y * 255 / 400), B: 180, A: 255})
		}
	}
	path := filepath.Join(t.TempDir(), "picture.png")
	file, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if err := png.Encode(file, picture); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestNewBotSheet(t *testing.T) {
	var created string
	m, tt := sheetTester(t, func(m *mainWindow) { m.presentNewBot(func(id string) { created = id }) })
	if !tt.HasText(L("%@ is not connected yet. The bot is created now and its first turn waits until you connect it in Settings.", "DeepSeek")) &&
		!tt.HasText(L("%@ is connected. Turns run on %@.", "DeepSeek", "Workbench")) {
		t.Errorf("no note: %q", tt.Texts())
	}
	// Create Bot waits for a name.
	if err := tt.Click(L("Create Bot")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if created != "" || !m.hasSheet() {
		t.Fatal("created a bot without a name")
	}
	if !tt.Focused(L("Name")) {
		t.Error("the name field does not have the focus")
	}
	if err := tt.Click("binoculars.fill"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	// Back to the name field, right of its label.
	label, _ := tt.Find(L("Name"))
	tt.ClickAt(label.X+newBotLabelWidth+60, label.Y+label.H/2)
	tt.Type("Analyst")
	tt.Key(0, ui.KeyTab)
	tt.Type("Reads the numbers.")
	settle(tt)
	renderTo(t, tt, "sheet-newbot-filled")
	// Return in the description creates the bot.
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if created == "" || m.hasSheet() {
		t.Fatalf("Return did not create the bot (sheet up: %v)", m.hasSheet())
	}
	bot := store.Bot(created)
	if bot == nil || bot.Name != "Analyst" || bot.Description != "Reads the numbers." || bot.SymbolName != "binoculars.fill" || bot.Accent != "teal" {
		t.Errorf("bot %+v", bot)
	}
	if bot != nil && (bot.RunnerID != "dev-workbench" || bot.Provider != model.ProviderKinds[0] || bot.Model != "" || bot.Thinking != "") {
		t.Errorf("bot runs %q on %q with %q/%q", bot.Provider, bot.RunnerID, bot.Model, bot.Thinking)
	}
}

// New Bot offers no decision provider, and no decision model of a provider that has some.
func TestNewBotLeavesDecisionModelsOut(t *testing.T) {
	_, tt := sheetTester(t, func(m *mainWindow) { m.presentNewBot(nil) })
	if err := tt.Click("DeepSeek (API key)"); err != nil {
		t.Fatalf("%v in %q", err, tt.Texts())
	}
	if got := tt.Menu(); slices.Contains(got, "OpenRouter Decisions (Custom)") || !slices.Contains(got, "Ollama (Custom)") {
		t.Errorf("providers %q", got)
	}
	if err := tt.ChooseMenuItem("OpenCode Zen (API key)"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click(L("Default (%@)", "DeepSeek V4.1 Flash")); err != nil {
		t.Fatalf("%v in %q", err, tt.Texts())
	}
	if got := tt.Menu(); slices.Contains(got, "Jev 1.13") || slices.Contains(got, "Jev 1.13 Free") || len(got) != 4 {
		t.Errorf("models %q", got)
	}
}

func TestBotLookSheet(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) { m.presentBotLook("bot-quill") })
	if tt.HasText(L("Remove Image")) {
		t.Error("Remove Image without an image")
	}
	if err := tt.Click("flask.fill"); err != nil {
		t.Fatal(err)
	}
	if err := tt.Click(L("Green")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderTo(t, tt, "sheet-look-picked")
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	bot := store.Bot("bot-quill")
	if m.hasSheet() || bot.SymbolName != "flask.fill" || bot.Accent != "green" {
		t.Errorf("look %q %q, sheet up %v", bot.SymbolName, bot.Accent, m.hasSheet())
	}
}

func TestPairingSheet(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) { m.presentPairing() })
	if !tt.HasText("lorca://pair?relay=https%3A%2F%2Florca.app&id=idk_9f2c41ab&ek=ek_57ca0d3b&n=482913") {
		t.Errorf("no pairing string: %q", tt.Texts())
	}
	if _, ok := tt.Find(L("Copy pairing string")); !ok {
		t.Error("no copy button")
	}
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	if m.hasSheet() {
		t.Error("Escape left the sheet up")
	}
}

func TestRoutineSheet(t *testing.T) {
	var edit string
	m, tt := sheetTester(t, func(m *mainWindow) {
		m.presentRoutine("rt-brief", store.Bot("bot-nova"), func(text string) { edit = text })
	})
	if !tt.HasText(L("On")) || !tt.HasText(L("Next run")) {
		t.Errorf("texts %q", tt.Texts())
	}
	if err := tt.Click(L("Pause")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Routine("rt-brief").IsEnabled || !tt.HasText(L("Resume")) || !tt.HasText(L("Paused")) {
		t.Error("Pause did not pause")
	}
	if err := tt.Click(L("Delete…")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderTo(t, tt, "sheet-routine-delete")
	if err := tt.Click(L("Cancel")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Routine("rt-brief") == nil || !m.hasSheet() {
		t.Fatal("Cancel deleted the routine")
	}
	if err := tt.Click(L("Edit in Chat…")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if edit != L("Edit my routine \"%@\": ", "Morning brief") || m.hasSheet() {
		t.Errorf("edit %q, sheet up %v", edit, m.hasSheet())
	}

	// Deleting closes the sheet, as the routine is gone.
	m.presentRoutine("rt-reviews", store.Bot("bot-nova"), nil)
	settle(tt)
	if err := tt.Click(L("Delete…")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click(L("Delete routine")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Routine("rt-reviews") != nil || m.hasSheet() {
		t.Errorf("routine left %v, sheet up %v", store.Routine("rt-reviews") != nil, m.hasSheet())
	}
}

func TestMemorySheet(t *testing.T) {
	saved := false
	var memory model.BotMemory
	m, tt := sheetTester(t, func(m *mainWindow) {
		memory = demoMemory(t, "bot-nova")
		m.presentMemory(store.Bot("bot-nova"), memory, func() { saved = true })
	})
	if !tt.HasText(L("%d lines of %d · %@ of %@", 3, memory.MaxLines, model.Kilobytes(len(memory.Text)), model.Kilobytes(memory.MaxBytes))) {
		t.Errorf("gauge: %q", tt.Texts())
	}
	tt.Type("- one more\n")
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !saved || m.hasSheet() {
		t.Errorf("saved %v, sheet up %v", saved, m.hasSheet())
	}
}

func TestMemoryGauge(t *testing.T) {
	memory := model.BotMemory{MaxLines: 10, MaxBytes: 1000}
	tests := []struct {
		text  string
		lines int
		tone  model.Tone
	}{
		{"", 0, model.ToneSecondary},
		{"a", 1, model.ToneSecondary},
		{"1\n2\n3\n4\n5\n6\n7\n8", 8, model.ToneOrange},
		{"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11", 11, model.ToneRed},
	}
	for _, test := range tests {
		text, tone := memoryGauge(test.text, memory)
		if tone != test.tone {
			t.Errorf("%q: tone %v, want %v (%s)", test.text, tone, test.tone, text)
		}
	}
	if text, _ := memoryGauge("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12", memory); text != L("%d lines of %d · %@ of %@", 12, 10, model.Kilobytes(26), model.Kilobytes(1000))+" · "+L("%d lines past the budget will not load", 2) {
		t.Errorf("over: %q", text)
	}
}

func TestNewGroupChatSheet(t *testing.T) {
	var picked []string
	var title string
	m, tt := sheetTester(t, func(m *mainWindow) {
		m.presentNewGroupChat(func(ids []string, name string) { picked, title = ids, name })
	})
	// The first bot starts picked; a second click on one drops it.
	for _, row := range []int{1, 2, 3, 4, 3} {
		clickPickerRow(t, tt, L("Bots"), row)
		settle(tt)
	}
	if err := tt.Click(L("Group name (optional)")); err != nil {
		t.Fatal(err)
	}
	tt.Type("  Ops  ")
	renderTo(t, tt, "sheet-newgroup-picked")
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	want := []string{"bot-nova", "bot-patch", "bot-scout", "bot-ember"}
	if m.hasSheet() || len(picked) != len(want) || title != "Ops" {
		t.Fatalf("picked %v %q, sheet up %v", picked, title, m.hasSheet())
	}
	for i := range want {
		if picked[i] != want[i] {
			t.Errorf("picked %v, want %v", picked, want)
		}
	}
}

// clickPickerRow clicks the row at `index` of a picker's section: the sidebar behind the sheet shows
// the same names, so the rows are found by their place under the section's caption.
func clickPickerRow(t *testing.T, tt *ui.Tester, section string, index int) {
	t.Helper()
	r, ok := tt.Find(strings.ToUpper(section))
	if !ok {
		t.Fatalf("no section %q", section)
	}
	top := r.Y + r.H/2 + 7 + 6
	tt.ClickAt(r.X+100, top+46*float32(index)+23)
}

func TestBotPickerSheet(t *testing.T) {
	var picked string
	_, tt := sheetTester(t, func(m *mainWindow) {
		m.presentBotPicker("Add", []*model.Bot{store.Bot("bot-patch"), store.Bot("bot-ember")}, func(id string) { picked = id })
	})
	clickPickerRow(t, tt, L("Available"), 1)
	settle(tt)
	renderTo(t, tt, "sheet-botpicker-picked")
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if picked != "bot-ember" {
		t.Errorf("picked %q", picked)
	}
}

func TestDescriptionSheet(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) { m.presentGroupDescription("chat-relay") })
	tt.Type("Ship the launch.\n")
	settle(tt)
	if !m.hasSheet() {
		t.Fatal("Return closed the sheet")
	}
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if got := store.Chat("chat-relay").GroupDescription; m.hasSheet() || got == "" || got[len(got)-len("Ship the launch."):] != "Ship the launch." {
		t.Errorf("description %q, sheet up %v", got, m.hasSheet())
	}
}
