package model

import (
	"reflect"
	"testing"
)

func TestPatchLinesNumberEachLine(t *testing.T) {
	diff := "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -3,3 +3,4 @@ fn main\n keep\n-old\n+new\n+more\n tail\n\\ No newline at end of file\n"
	want := []PatchLine{
		{Kind: PatchHunk, Text: "@@ -3,3 +3,4 @@ fn main"},
		{Kind: PatchContext, Text: "keep", Old: 3, New: 3},
		{Kind: PatchRemoved, Text: "old", Old: 4},
		{Kind: PatchAdded, Text: "new", New: 4},
		{Kind: PatchAdded, Text: "more", New: 5},
		{Kind: PatchContext, Text: "tail", Old: 5, New: 6},
		{Kind: PatchNote, Text: "No newline at end of file"},
	}
	if got := PatchLines(diff); !reflect.DeepEqual(got, want) {
		t.Errorf("got %+v", got)
	}
	if got := PatchLines("--- /dev/null\n+++ b/logo.png\nBinary file\n"); !reflect.DeepEqual(got, []PatchLine{{Kind: PatchNote, Text: "Binary file"}}) {
		t.Errorf("binary %+v", got)
	}
	if got := PatchLines(""); len(got) != 0 {
		t.Errorf("empty %+v", got)
	}
}

func TestWorkdirChangeNames(t *testing.T) {
	change := WorkdirChange{Path: "src/cart/total.ts", Status: "untracked"}
	if change.Name() != "total.ts" || change.Folder() != "src/cart" || change.Mark() != "U" {
		t.Errorf("%q %q %q", change.Name(), change.Folder(), change.Mark())
	}
	if top := (WorkdirChange{Path: "README.md", Status: "deleted"}); top.Folder() != "" || top.Mark() != "D" {
		t.Errorf("%q %q", top.Folder(), top.Mark())
	}
}
