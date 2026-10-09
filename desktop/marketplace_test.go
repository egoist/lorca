package main

import (
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// marketplaceTester opens the marketplace over the demo's main window. With `loaded`, the catalog
// has come in.
func marketplaceTester(t *testing.T, loaded bool) (*mainWindow, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	tt.Frame()
	m.presentMarketplace("")
	tt.Frame()
	if loaded {
		runPosts()
	}
	for range 3 {
		tt.Frame()
	}
	return m, tt
}

// marketClick clicks the element with `text`, runs what the store posted, and settles.
func marketClick(t *testing.T, tt *ui.Tester, text string) {
	t.Helper()
	// New leading sections can put a catalog entry below the viewport. Scroll the
	// actual native sheet before clicking, as a user does.
	for range 16 {
		rect, found := tt.Find(text)
		if !found || (rect.Y >= 100 && rect.Y+rect.H <= 700) {
			break
		}
		tt.Scroll(590, 380, 0, rect.Y-340)
		tt.Frame()
	}
	if err := tt.Click(text); err != nil {
		t.Fatalf("click %q: %v; texts %q", text, err, tt.Texts())
	}
	runPosts()
	tt.Frame()
	tt.Frame()
}

func wantText(t *testing.T, tt *ui.Tester, texts ...string) {
	t.Helper()
	for _, text := range texts {
		if !tt.HasText(text) {
			t.Errorf("no %q; texts %q", text, tt.Texts())
		}
	}
}

func TestRenderMarketplace(t *testing.T) {
	_, tt := marketplaceTester(t, true)
	wantText(t, tt, L("Featured Plugins"), L("Featured Bots"), L("Productivity"), L("%d installed", 5))
	if !tt.Focused(L("Search plugins and bots")) {
		t.Errorf("the search does not have the keyboard as the sheet opens")
	}
	renderBoth(t, tt, "market-home")

	// The curated index can add featured entries ahead of GitHub. Find the installed plugin
	// through the same search users have, rather than depending on the first preview rows.
	marketClick(t, tt, L("Search plugins and bots"))
	tt.Type("github")
	tt.Frame()
	marketClick(t, tt, "GitHub")
	wantText(t, tt, L("Manage…"), L("On %@", "Workbench"), L("Servers"), L("Information"))
	renderBoth(t, tt, "market-plugin-github")
	marketClick(t, tt, L("Back"))
	marketClick(t, tt, L("Search plugins and bots"))
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Key(0, ui.KeyBackspace)
	tt.Frame()

	marketClick(t, tt, L("View all"))
	wantText(t, tt, L("Plugins"), "Granola")
	renderBoth(t, tt, "market-list")
	marketClick(t, tt, L("Back"))

	marketClick(t, tt, L("%d installed", 5))
	wantText(t, tt, L("Plugins on %@", "Workbench"), "deepwiki")
	renderBoth(t, tt, "market-installed")
	marketClick(t, tt, L("Back"))

	marketClick(t, tt, L("Search plugins and bots"))
	tt.Type("linear")
	tt.Frame()
	wantText(t, tt, L("Results"), L("All"), "Issue Triager")
	renderBoth(t, tt, "market-search")
	marketClick(t, tt, L("Bots"))
	if tt.HasText("Connect") {
		t.Errorf("the Bots filter still shows the Linear plugin")
	}
	marketClick(t, tt, L("Search plugins and bots"))
	tt.Type("zzz")
	tt.Frame()
	wantText(t, tt, L("No results match “%@”", "linearzzz"))
}

// issueTriager is the Issue Triager bot's row: its line about it.
const issueTriager = "Sorts new Linear issues, fills in missing details, and flags what needs you"

func TestRenderMarketplaceBot(t *testing.T) {
	_, tt := marketplaceTester(t, true)
	marketClick(t, tt, issueTriager)
	wantText(t, tt, L("Add Bot"), L("Instructions"), L("How this bot should work"))
	renderBoth(t, tt, "market-bot")
	for _, part := range []string{"Routines", "Plugins", "Memories"} {
		if _, ok := tt.Find(L(part)); !ok {
			continue
		}
		marketClick(t, tt, L(part))
		renderBoth(t, tt, "market-bot-"+part)
	}
}

func TestRenderMarketplaceLoading(t *testing.T) {
	_, tt := marketplaceTester(t, false)
	wantText(t, tt, L("Loading the marketplace…"))
	renderBoth(t, tt, "market-loading")
}

func TestMarketplaceInstall(t *testing.T) {
	_, tt := marketplaceTester(t, true)
	marketClick(t, tt, "Notion")
	if err := tt.Click(L("Add")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText(L("Notion")) || tt.HasText(L("Manage…")) {
		t.Errorf("installed before the Runner answered; texts %q", tt.Texts())
	}
	runPosts()
	tt.Frame()
	wantText(t, tt, L("Manage…"), L("Added %@. Every bot on %@ can use it.", "notion", "Workbench"))
	renderBoth(t, tt, "market-installed-notice")
	marketClick(t, tt, L("Back"))
	if _, ok := tt.Find(L("%d installed", 6)); !ok {
		t.Errorf("the install is not counted; texts %q", tt.Texts())
	}
}

func TestMarketplaceRunnerPicker(t *testing.T) {
	_, tt := marketplaceTester(t, true)
	if err := tt.Click(L("Plugins install here, and bots are added here.")); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem("Studio"); err != nil {
		t.Fatalf("%v; menu %q", err, tt.Menu())
	}
	tt.Frame()
	tt.Frame()
	if _, ok := tt.Find(L("%d installed", 0)); !ok {
		t.Errorf("Studio's plugins are not shown; texts %q", tt.Texts())
	}
	if tt.HasText(L("Added")) {
		t.Errorf("GitHub shows as added on Studio")
	}
}

func TestMarketplaceOnRunner(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	m.presentMarketplace("dev-studio")
	runPosts()
	tt.Frame()
	tt.Frame()
	if _, ok := tt.Find(L("%d installed", 0)); !ok || !tt.HasText("Studio") {
		t.Errorf("the marketplace does not open on Studio; texts %q", tt.Texts())
	}
	// The pointer fills the row under it.
	if r, ok := tt.Find("Notion"); ok {
		tt.Move(r.X+2, r.Y+2)
		tt.Frame()
		renderTo(t, tt, "market-hover-light")
	}
}

func TestMarketplaceAddBot(t *testing.T) {
	m, tt := marketplaceTester(t, true)
	before := len(store.Bots)
	marketClick(t, tt, issueTriager)
	marketClick(t, tt, L("Add Bot"))
	if m.hasSheet() {
		t.Errorf("the sheet stays up after adding a bot")
	}
	if len(store.Bots) != before+1 {
		t.Fatalf("bots %d, want %d", len(store.Bots), before+1)
	}
	chat := store.Chat(m.selection.ChatID)
	if chat == nil || store.Title(chat) != "Issue Triager" {
		t.Errorf("the new bot's chat is not selected: %+v", m.selection)
	}
}

func TestMarketplaceEscape(t *testing.T) {
	m, tt := marketplaceTester(t, true)
	marketClick(t, tt, L("Search plugins and bots"))
	tt.Type("git")
	tt.Frame()
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if m.hasSheet() {
		t.Errorf("Escape in the search does not close the sheet")
	}
	// It opens again, once.
	m.presentMarketplace("")
	m.presentMarketplace("")
	if len(m.sheets) != 1 {
		t.Errorf("sheets %d, want 1", len(m.sheets))
	}
}

// A drag from a bot's first routine prompt to the note below copies the prompts, without the
// names, schedules, or note.
func TestMarketplaceBotPartSelects(t *testing.T) {
	_, tt := marketplaceTester(t, true)
	marketClick(t, tt, issueTriager)
	marketClick(t, tt, L("Routines"))
	var template model.BotTemplate
	store.Marketplace(func(index model.Marketplace, err error) {
		for _, bot := range index.Bots {
			if bot.Summary == issueTriager {
				template = bot
			}
		}
	})
	runPosts()
	if len(template.Routines) == 0 {
		t.Fatal("Issue Triager has no routines")
	}
	first, ok := tt.Find(template.Routines[0].Prompt)
	note, ok2 := tt.Find(L("They start paused. %@ asks whether to turn them on.", template.Name))
	if !ok || !ok2 {
		t.Fatalf("no routine or note: %q", tt.Texts())
	}
	tt.Press(first.X+1, first.Y+first.H/2)
	tt.Move(note.X+note.W/2, note.Y+note.H/2)
	tt.Release(note.X+note.W-1, note.Y+note.H/2)
	tt.Key(ui.Cmd, ui.KeyC)
	var prompts []string
	for _, routine := range template.Routines {
		prompts = append(prompts, routine.Prompt)
	}
	if got, want := tt.Clipboard(), strings.Join(prompts, "\n"); got != want {
		t.Errorf("copied %q, want %q", got, want)
	}
}
