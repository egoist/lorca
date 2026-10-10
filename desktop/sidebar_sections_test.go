package main

import (
	"slices"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// rowNames are the rows as words: a chat's id, a group's header in brackets.
func rowNames(rows []sidebarRow) []string {
	var names []string
	for _, row := range rows {
		if row.chat != nil {
			names = append(names, row.chat.ID)
		} else {
			names = append(names, "["+groupTitle(*row.group)+"]")
		}
	}
	return names
}

func TestSidebarRowsListSectionsThenTheRestThenHidden(t *testing.T) {
	demoWindow(t)
	chat := func(id, section string, pinned, hidden bool) *model.Chat {
		return &model.Chat{ID: id, SectionID: section, IsPinned: pinned, IsHidden: hidden}
	}
	chats := []*model.Chat{chat("pinned", "s1", true, false), chat("deal", "s1", false, false), chat("loose", "", false, false), chat("orphan", "deleted", false, false), chat("gone", "s1", false, true)}
	store.Sections = []*model.Section{{ID: "s1", Name: "Pipeline"}, {ID: "s2", Name: "Empty", Collapsed: true}}
	got := rowNames(sidebarRows(chats, store.Sections, false, false))
	want := []string{"pinned", "[Pipeline]", "deal", "[Empty]", "[Chats]", "loose", "orphan", "[Hidden]"}
	if !slices.Equal(got, want) {
		t.Fatalf("rows %q, want %q", got, want)
	}
	if got := rowNames(sidebarRows(chats[:3], nil, false, false)); !slices.Equal(got, []string{"pinned", "deal", "loose"}) {
		t.Fatalf("without sections the chats are one list: %q", got)
	}
	if got := rowNames(sidebarRows(chats[:2], store.Sections, true, false)); slices.Contains(got, "[Chats]") || slices.Contains(got, "[Hidden]") {
		t.Fatalf("an empty group shows nothing: %q", got)
	}
}

func TestSidebarSectionsFoldMoveMuteAndHide(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	for range 4 {
		tt.Frame()
	}
	rows := func() []string {
		pr := prefs.get()
		return rowNames(sidebarRows(store.Chats, store.Sections, pr.ShowsHiddenChats, pr.CollapsesOtherChats))
	}
	// The demo's activity sets the order within a group; the groups and their chats are fixed.
	got := rows()
	product, engineering, chats := slices.Index(got, "[Product]"), slices.Index(got, "[Engineering]"), slices.Index(got, "[Chats]")
	if got[0] != "chat-relay" || product != 1 || engineering != 4 || chats != 7 {
		t.Fatalf("rows %q: want the pinned chat, then Product, Engineering, and Chats with two chats in each section", got)
	}
	if section := got[2:4]; !slices.Contains(section, "chat-nova") || !slices.Contains(section, "chat-launch") {
		t.Fatalf("Product holds %q", section)
	}
	if section := got[5:7]; !slices.Contains(section, "chat-patch") || !slices.Contains(section, "chat-ember") {
		t.Fatalf("Engineering holds %q", section)
	}
	if !tt.HasText("Product") || !tt.HasText(Lc("Chats", "no section")) {
		t.Fatalf("no headers: %q", tt.Texts())
	}
	renderBoth(t, tt, "sidebar-sections")

	// The header's chevron shows under the pointer and folds the section on every Device.
	header, ok := tt.Find("Product")
	if !ok {
		t.Fatal("no Product header")
	}
	x, y := header.X+header.W-15, header.Y+header.H-14
	tt.Move(x, y)
	tt.Frame()
	tt.ClickAt(x, y)
	tt.Frame()
	if section := store.Section("section-product"); section == nil || !section.Collapsed {
		t.Fatal("the chevron did not fold the section")
	}
	if got := rows(); slices.Contains(got, "chat-nova") {
		t.Fatalf("a folded section lists its chats: %q", got)
	}
	if chats := visibleChats(); chats[1].ID != "chat-patch" {
		t.Fatalf("Ctrl+2 opens %q, want the next chat that shows", chats[1].ID)
	}
	store.SetSectionCollapsed("section-product", false)
	tt.Frame()

	// A row's menu files, mutes, and hides its chat.
	if err := tt.RightClick("Writer"); err != nil {
		t.Fatal(err)
	}
	if menu := tt.Menu(); !slices.Contains(menu, L("Mute")) || !slices.Contains(menu, L("Move to Section")) || !slices.Contains(menu, L("Hide")) {
		t.Fatalf("the row's menu is %q", menu)
	}
	if err := tt.ChooseMenuItem(L("Move to Section"), "Engineering"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if got := store.Chat("chat-quill").SectionID; got != "section-engineering" {
		t.Fatalf("Writer is in %q", got)
	}
	if err := tt.RightClick("Developer"); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem(L("Mute"), L("For 1 Hour")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if chat := store.Chat("chat-patch"); !chat.IsMuted(time.Now()) || chat.IsMuted(time.Now().Add(61*time.Minute)) {
		t.Fatal("Developer is not muted for an hour")
	}
	if err := tt.RightClick("Researcher"); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem(L("Hide")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if got := rows(); got[len(got)-1] != "[Hidden]" || slices.Contains(got, "chat-scout") {
		t.Fatalf("Researcher is not folded away under Hidden: %q", got)
	}

	// A section's header moves it.
	if err := tt.RightClick("Engineering"); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem(L("Move Up")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if store.Sections[0].ID != "section-engineering" {
		t.Fatalf("sections %v", store.Sections)
	}

	// Opening a hidden chat some other way unfolds Hidden to show its row.
	m.open("chat-scout")
	tt.Frame()
	tt.Frame()
	if got := rows(); got[len(got)-1] != "chat-scout" || !prefs.get().ShowsHiddenChats {
		t.Fatalf("Hidden did not unfold: %q", got)
	}
	settle(tt)
	renderBoth(t, tt, "sidebar-sections-hidden")
}
