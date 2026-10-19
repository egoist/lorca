package main

import (
	"slices"
	"testing"

	"github.com/egoist/mygo/ui"
)

// A DM's inspector shows what changed in the bot's working folder, each file a click away from its
// diff, and the folder's files as a tree that lists a folder when it opens; the Profile's Working
// Folder row picks the folder.
func TestInspectorWorkingFolder(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-nova")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	for _, text := range []string{L("Changes on %@", "main"), "checkout.ts", "src", "package.json", "~/Projects/shop"} {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
	renderTo(t, tt, "inspector-workdir")

	// The tree on its own, where src is the folder's row alone.
	files := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		m.inspectorFiles(c, store.Bot("bot-nova"))
	}, 300, 400)
	settle(files)
	if slices.Contains(files.Texts(), "cart") {
		t.Fatal("a folder lists its files only once it opens")
	}
	if err := files.Click("src"); err != nil {
		t.Fatal(err)
	}
	settle(files)
	if !slices.Contains(files.Texts(), "cart") || !slices.Contains(files.Texts(), "checkout.ts") {
		t.Fatalf("src did not open: %q", files.Texts())
	}
	if err := files.Click("cart"); err != nil {
		t.Fatal(err)
	}
	settle(files)
	if !slices.Contains(files.Texts(), "total.ts") {
		t.Fatalf("cart did not open: %q", files.Texts())
	}
	renderTo(t, files, "inspector-workdir-tree")
	if err := files.Click("src"); err != nil {
		t.Fatal(err)
	}
	settle(files)
	if slices.Contains(files.Texts(), "cart") {
		t.Fatal("src did not close")
	}

	if err := tt.Click("src/checkout.ts"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !m.hasSheet() || !tt.HasText("const shipping = 0") {
		t.Fatalf("no diff sheet: %q", tt.Texts())
	}
	renderTo(t, tt, "sheet-workdir-diff")
	m.sheets[len(m.sheets)-1].dismiss()
	settleTransitions(tt)

	if err := tt.Click(L("Working Folder")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !m.hasSheet() {
		t.Fatal("Working Folder opened no sheet")
	}
	tt.Type("~/Projects/shop")
	tt.Frame()
	renderTo(t, tt, "sheet-working-folder")
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if got := store.Bot("bot-nova").Workdir; got != "~/Projects/shop" || m.hasSheet() {
		t.Fatalf("workdir %q, sheet up %v", got, m.hasSheet())
	}
	if !tt.HasText("~/Projects/shop") {
		t.Fatal("the Profile row does not show the folder")
	}
}
