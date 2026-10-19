package model

import (
	"path"
	"strings"
)

// A bot's working folder as its Runner reads it: the files in it a folder at a time, and in a git
// repository what changed there since the last commit and each file's diff
// (`bots.workdir.files`, `bots.workdir.changes`, `bots.workdir.diff`).

// WorkdirEntry is a file or a folder in a bot's working folder.
type WorkdirEntry struct {
	Name string `json:"name"`
	// Path is from the working folder, with `/` between its parts.
	Path string `json:"path"`
	Dir  bool   `json:"dir"`
}

// WorkdirFiles is one folder of a bot's working folder.
type WorkdirFiles struct {
	// Folder is the working folder as the user reads it (`~/Projects/shop`), Path where it is.
	Folder  string         `json:"folder"`
	Path    string         `json:"path"`
	Entries []WorkdirEntry `json:"entries"`
	// More is how many entries past the ones listed.
	More int `json:"more"`
	// Here is the bot running on this computer, whose files open here; Runner names its Runner.
	Here   bool   `json:"here"`
	Runner string `json:"runner"`
}

// WorkdirChange is a file that changed since the last commit.
type WorkdirChange struct {
	Path string `json:"path"`
	// Status is `modified`, `added`, `deleted`, or `untracked`.
	Status  string `json:"status"`
	Added   int    `json:"added"`
	Removed int    `json:"removed"`
}

// Name is the file's name, and Folder the folder it is in, empty at the top.
func (c WorkdirChange) Name() string { return path.Base(c.Path) }

func (c WorkdirChange) Folder() string {
	if folder := path.Dir(c.Path); folder != "." {
		return folder
	}
	return ""
}

// Mark is the change as a letter, as git's status writes it.
func (c WorkdirChange) Mark() string {
	switch c.Status {
	case "added":
		return "A"
	case "deleted":
		return "D"
	case "untracked":
		return "U"
	}
	return "M"
}

// WorkdirChanges is what changed in a bot's working folder.
type WorkdirChanges struct {
	// Repository is the folder being in a git repository; outside one there is nothing else.
	Repository bool            `json:"repository"`
	Branch     string          `json:"branch"`
	Files      []WorkdirChange `json:"files"`
	More       int             `json:"more"`
	Here       bool            `json:"here"`
}

// Totals are the lines added and removed across the files.
func (c WorkdirChanges) Totals() (added, removed int) {
	for _, file := range c.Files {
		added += file.Added
		removed += file.Removed
	}
	return
}

// WorkdirDiff is one file's diff, as git writes it.
type WorkdirDiff struct {
	Diff      string `json:"diff"`
	Truncated bool   `json:"truncated"`
}

// PatchLine is a line of a file's diff: a hunk's header, a line around the change, or one taken
// out or put in.
type PatchLine struct {
	Kind PatchKind
	Text string
	// Old and New are the line's numbers before and after, zero where it has none.
	Old, New int
}

type PatchKind int

const (
	PatchContext PatchKind = iota
	PatchAdded
	PatchRemoved
	PatchHunk
	// PatchNote is what git says that is no line of the file: a binary file, no newline at the end.
	PatchNote
)

// PatchLines are the lines of a file's diff, without git's headers before its first hunk.
func PatchLines(diff string) []PatchLine {
	var lines []PatchLine
	old, new := 0, 0
	inHunk := false
	for _, line := range strings.Split(strings.TrimSuffix(diff, "\n"), "\n") {
		switch {
		case strings.HasPrefix(line, "@@"):
			inHunk = true
			old, new = hunkStart(line)
			lines = append(lines, PatchLine{Kind: PatchHunk, Text: line})
		case !inHunk:
			if strings.HasPrefix(line, "Binary file") {
				lines = append(lines, PatchLine{Kind: PatchNote, Text: line})
			}
		case strings.HasPrefix(line, "+"):
			lines = append(lines, PatchLine{Kind: PatchAdded, Text: line[1:], New: new})
			new++
		case strings.HasPrefix(line, "-"):
			lines = append(lines, PatchLine{Kind: PatchRemoved, Text: line[1:], Old: old})
			old++
		case strings.HasPrefix(line, `\`):
			lines = append(lines, PatchLine{Kind: PatchNote, Text: strings.TrimSpace(line[1:])})
		default:
			lines = append(lines, PatchLine{Kind: PatchContext, Text: strings.TrimPrefix(line, " "), Old: old, New: new})
			old++
			new++
		}
	}
	return lines
}

// hunkStart reads `@@ -12,3 +12,4 @@` for the first line numbers it starts at.
func hunkStart(header string) (old, new int) {
	for _, field := range strings.Fields(header) {
		if len(field) < 2 || (field[0] != '-' && field[0] != '+') {
			continue
		}
		start, _, _ := strings.Cut(field[1:], ",")
		n := 0
		for _, r := range start {
			if r < '0' || r > '9' {
				n = 0
				break
			}
			n = n*10 + int(r-'0')
		}
		if field[0] == '-' {
			old = n
		} else {
			new = n
		}
	}
	return
}

// WorkdirFiles asks the bot's Runner for the folder at `dir` in its working folder, the top when
// empty.
func (s *Store) WorkdirFiles(botID, dir string, done func(WorkdirFiles, error)) {
	if s.IsMock {
		files := mockWorkdirFiles(dir)
		s.post(func() { done(files, nil) })
		return
	}
	Async(s, func() (WorkdirFiles, error) {
		return call[WorkdirFiles](s, "bots.workdir.files", map[string]any{"bot_id": botID, "path": dir})
	}, done)
}

// WorkdirChanges asks the bot's Runner what changed in its working folder.
func (s *Store) WorkdirChanges(botID string, done func(WorkdirChanges, error)) {
	if s.IsMock {
		changes := WorkdirChanges{Repository: true, Branch: "main", Here: true, Files: []WorkdirChange{
			{Path: "src/checkout.ts", Status: "modified", Added: 12, Removed: 4},
			{Path: "src/cart/total.ts", Status: "untracked", Added: 31},
			{Path: "README.md", Status: "modified", Added: 2, Removed: 1},
		}}
		s.post(func() { done(changes, nil) })
		return
	}
	Async(s, func() (WorkdirChanges, error) {
		return call[WorkdirChanges](s, "bots.workdir.changes", map[string]any{"bot_id": botID})
	}, done)
}

// WorkdirDiff asks the bot's Runner for one file's diff since the last commit.
func (s *Store) WorkdirDiff(botID, file string, done func(WorkdirDiff, error)) {
	if s.IsMock {
		diff := WorkdirDiff{Diff: "--- a/" + file + "\n+++ b/" + file + "\n@@ -1,4 +1,5 @@\n import { cart } from \"./cart\"\n-const total = cart.sum()\n+const total = cart.sum({ tax: true })\n+const shipping = 0\n export { total }\n"}
		s.post(func() { done(diff, nil) })
		return
	}
	Async(s, func() (WorkdirDiff, error) {
		return call[WorkdirDiff](s, "bots.workdir.diff", map[string]any{"bot_id": botID, "path": file})
	}, done)
}

func mockWorkdirFiles(dir string) WorkdirFiles {
	files := WorkdirFiles{Folder: "~/Projects/shop", Path: "/Users/demo/Projects/shop", Here: true, Runner: "This computer"}
	switch dir {
	case "":
		files.Entries = []WorkdirEntry{{Name: "src", Path: "src", Dir: true}, {Name: "package.json", Path: "package.json"}, {Name: "README.md", Path: "README.md"}}
	case "src":
		files.Entries = []WorkdirEntry{{Name: "cart", Path: "src/cart", Dir: true}, {Name: "checkout.ts", Path: "src/checkout.ts"}}
	case "src/cart":
		files.Entries = []WorkdirEntry{{Name: "total.ts", Path: "src/cart/total.ts"}}
	}
	return files
}
