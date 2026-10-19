package main

import (
	"fmt"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// A DM bot's working folder in the inspector: the folder the user picked for it on its Runner (its
// own workspace until then), what changed there since the last commit with each file's diff, and
// its files as a tree that lists a folder when it opens. The Runner reads them; a bot on another
// Runner is read through the relay, and only its files cannot be opened here.

// workdirChangesShown is how many changed files the section lists before Show More.
const workdirChangesShown = 8

// workdirView is what a bot's Runner last said about its working folder.
type workdirView struct {
	// root is the folder's own listing; folders holds every folder listed, the top under "".
	root     *model.WorkdirFiles
	folders  map[string][]model.WorkdirEntry
	more     map[string]int
	expanded map[string]bool
	loading  map[string]bool
	err      string
	changes  *model.WorkdirChanges
	// showingAll is the Changes section listing every file.
	showingAll bool
}

func (s *inspectorState) workdirOf(botID string) *workdirView {
	if s.workdir == nil {
		s.workdir = map[string]*workdirView{}
	}
	v := s.workdir[botID]
	if v == nil {
		v = &workdirView{folders: map[string][]model.WorkdirEntry{}, more: map[string]int{}, expanded: map[string]bool{}, loading: map[string]bool{}}
		s.workdir[botID] = v
	}
	return v
}

// refreshWorkdir asks the bot's Runner again for its working folder's top, every folder open in
// the tree, and what changed.
func (s *inspectorState) refreshWorkdir(botID string) {
	v := s.workdirOf(botID)
	v.list(botID, "")
	for dir, open := range v.expanded {
		if open {
			v.list(botID, dir)
		}
	}
	store.WorkdirChanges(botID, func(changes model.WorkdirChanges, err error) {
		if err != nil {
			v.changes = nil
			return
		}
		v.changes = &changes
	})
}

// list asks for the folder at `dir`; the tree redraws when it answers.
func (v *workdirView) list(botID, dir string) {
	if v.loading[dir] {
		return
	}
	v.loading[dir] = true
	store.WorkdirFiles(botID, dir, func(files model.WorkdirFiles, err error) {
		delete(v.loading, dir)
		if err != nil {
			if dir == "" {
				v.root, v.err = nil, model.ErrorText(err)
				clear(v.folders)
			}
			return
		}
		if dir == "" {
			v.root, v.err = &files, ""
		}
		v.folders[dir], v.more[dir] = files.Entries, files.More
	})
}

// workdirLabel is the bot's working folder as the Profile row reads it.
func workdirLabel(bot *model.Bot) string {
	if bot.Workdir != "" {
		return bot.Workdir
	}
	return L("Its own workspace")
}

// inspectorChanges lists what changed in the bot's working folder since the last commit, a row per
// file that opens its diff. It is left out outside a git repository and while nothing changed.
func (m *mainWindow) inspectorChanges(c *ui.Context, bot *model.Bot) {
	v := m.inspector.workdirOf(bot.ID)
	changes := v.changes
	if changes == nil || !changes.Repository || len(changes.Files) == 0 {
		return
	}
	p := colors(c)
	files := changes.Files
	if !v.showingAll && len(files) > workdirChangesShown {
		files = files[:workdirChangesShown]
	}
	added, removed := changes.Totals()
	title := L("Changes")
	if changes.Branch != "" {
		title = L("Changes on %@", changes.Branch)
	}
	botID := bot.ID
	section(c.Key("workdir-changes"), title, sectionCaption, func() {
		ui.Row(c).Gap(4).Children(func() {
			ui.Text(c, fmt.Sprintf("+%d", added)).FontSize(10).FontWeight(600).TextColor(p.Green)
			ui.Text(c, fmt.Sprintf("−%d", removed)).FontSize(10).FontWeight(600).TextColor(p.Red)
		})
	}, func(k *card) {
		for _, file := range files {
			change := file
			ui.Box(c.Key(file.Path)).Children(func() {
				if workdirChangeRow(c, k, change) {
					m.presentWorkdirDiff(botID, change)
				}
			})
		}
		if hidden := len(changes.Files) - len(files) + changes.More; hidden > 0 && !v.showingAll {
			ui.Box(c.Key("all")).Children(func() {
				if taskRow(c, k, "ellipsis", p.Label3, L("Show %d More", hidden), "") {
					v.showingAll = true
				}
			})
		}
	})
}

// workdirChangeRow is a changed file: git's letter for the change, its name over its folder, and
// the lines it adds and removes. It answers a click.
func workdirChangeRow(c *ui.Context, k *card, change model.WorkdirChange) bool {
	p := colors(c)
	r := k.row(rowBox(c).Label(change.Path).Tooltip(change.Path).Cursor(ui.CursorPointer).Role(ui.RoleButton))
	if r.Hovered() {
		r.Background(p.RowHover)
	}
	clicked := r.Clicked()
	tint := p.Orange
	switch change.Status {
	case "added", "untracked":
		tint = p.Green
	case "deleted":
		tint = p.Red
	}
	r.Children(func() {
		ui.Text(c, change.Mark()).Width(12).Shrink(0).Font(monoFont).FontSize(11).FontWeight(600).TextColor(tint)
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, change.Name()).FontSize(12.5).SingleLine()
			if folder := change.Folder(); folder != "" {
				ui.Text(c, folder).FontSize(textCaption).TextColor(p.Label2).SingleLine()
			}
		})
		ui.Row(c).Shrink(0).Gap(4).Children(func() {
			if change.Added > 0 {
				ui.Text(c, fmt.Sprintf("+%d", change.Added)).Font(monoFont).FontSize(10.5).TextColor(p.Green)
			}
			if change.Removed > 0 {
				ui.Text(c, fmt.Sprintf("−%d", change.Removed)).Font(monoFont).FontSize(10.5).TextColor(p.Red)
			}
		})
	})
	return clicked
}

// inspectorFiles is the bot's working folder as a tree: a click opens or closes a folder, listing
// it the first time, and opens a file with its app when the bot runs on this computer.
func (m *mainWindow) inspectorFiles(c *ui.Context, bot *model.Bot) {
	p := colors(c)
	v := m.inspector.workdirOf(bot.ID)
	botID := bot.ID
	section(c.Key("workdir-files"), L("Files"), sectionCaption, nil, func(k *card) {
		if v.root == nil {
			if v.err != "" {
				if _, result := actionRow(c, k, L("Folder"), actionRowOptions{Value: v.err, Tint: &p.Label2, Action: L("Retry")}); result.Action {
					m.inspector.refreshWorkdir(botID)
				}
				return
			}
			keyValueRow(c, k, L("Folder"), L("Loading…"), false, &p.Label2)
			return
		}
		root := v.root
		action := ""
		if root.Here {
			action = L("Show")
		}
		folder := root.Folder
		if !root.Here {
			folder = L("%@ · on %@", root.Folder, root.Runner)
		}
		if _, result := actionRow(c.Key("folder"), k, L("Folder"), actionRowOptions{Value: folder, Tint: &p.Label2, Action: action, Tooltip: root.Path}); result.Action {
			openFile(root.Path)
		}
		if len(v.folders[""]) == 0 {
			noteRow(c, k, L("Nothing in this folder yet."), nil)
			return
		}
		m.workdirTree(c, k, botID, v, "", 0)
	})
}

// workdirTree is the rows of the folder at `dir` and of every folder open under it.
func (m *mainWindow) workdirTree(c *ui.Context, k *card, botID string, v *workdirView, dir string, depth int) {
	for _, entry := range v.folders[dir] {
		entry := entry
		ui.Box(c.Key(entry.Path)).Children(func() {
			if workdirEntryRow(c, k, entry, depth, v.expanded[entry.Path], v.root.Here) {
				switch {
				case entry.Dir && v.expanded[entry.Path]:
					delete(v.expanded, entry.Path)
				case entry.Dir:
					v.expanded[entry.Path] = true
					if _, listed := v.folders[entry.Path]; !listed {
						v.list(botID, entry.Path)
					}
				case v.root.Here:
					openFile(joinPath(v.root.Path, nativePath(v.root.Path, entry.Path)))
				}
			}
		})
		if entry.Dir && v.expanded[entry.Path] {
			if _, listed := v.folders[entry.Path]; !listed {
				ui.Box(c.Key(entry.Path + "/…")).Children(func() {
					workdirNoteRow(c, k, L("Loading…"), depth+1)
				})
				continue
			}
			m.workdirTree(c, k, botID, v, entry.Path, depth+1)
		}
	}
	if more := v.more[dir]; more > 0 {
		ui.Box(c.Key(dir + "/more")).Children(func() {
			workdirNoteRow(c, k, L("%d more not shown", more), depth)
		})
	}
}

// workdirEntryRow is a file or a folder in the tree, indented to its depth, a folder with its
// chevron. A file of a bot on another Runner takes no click.
func workdirEntryRow(c *ui.Context, k *card, entry model.WorkdirEntry, depth int, open, here bool) bool {
	p := colors(c)
	clickable := entry.Dir || here
	r := k.row(ui.Row(c).Gap(6).MinHeight(26).Padding(3, 12, 3, 12+float32(depth)*14).MinWidth(0).Label(entry.Name).Tooltip(entry.Path))
	if clickable {
		r.Cursor(ui.CursorPointer).Role(ui.RoleButton)
		if r.Hovered() {
			r.Background(p.RowHover)
		}
	}
	clicked := clickable && r.Clicked()
	r.Children(func() {
		ui.Row(c).Width(10).Shrink(0).Justify(ui.Center).TextColor(p.Label3).Children(func() {
			if entry.Dir {
				chevron := "chevron.right"
				if open {
					chevron = "chevron.down"
				}
				symbol(c, chevron, 10, 2.2)
			}
		})
		icon, tint := "doc", p.Label2
		if entry.Dir {
			icon, tint = "folder", p.Accent
		}
		ui.Row(c).Shrink(0).TextColor(tint).Children(func() { symbol(c, icon, 13, 1.8) })
		ui.Text(c, entry.Name).Grow(1).Shrink(1).MinWidth(0).FontSize(12).SingleLine()
	})
	return clicked
}

// workdirNoteRow is a quiet line in the tree: a folder on its way, or how many entries it left out.
func workdirNoteRow(c *ui.Context, k *card, text string, depth int) {
	p := colors(c)
	k.row(ui.Row(c).MinHeight(24).Padding(3, 12, 3, 12+float32(depth)*14+16).Children(func() {
		ui.Text(c, text).FontSize(textCaption).TextColor(p.Label2).SingleLine()
	}))
}

// nativePath is a tree path in the separators of the folder it is under.
func nativePath(root, path string) string {
	if strings.Contains(root, "\\") && !strings.Contains(root, "/") {
		return strings.ReplaceAll(path, "/", "\\")
	}
	return path
}

// presentWorkingFolder edits the folder on the bot's Runner where it mainly works. Choose… picks
// one when the bot runs on this computer; an empty field is its own workspace again.
func (m *mainWindow) presentWorkingFolder(botID string) {
	bot := store.Bot(botID)
	if bot == nil {
		return
	}
	folder := bot.Workdir
	runner := L("its Runner")
	if device := store.Device(bot.RunnerID); device != nil {
		runner = device.Name
	}
	this := store.ThisDevice()
	here := this != nil && this.ID == bot.RunnerID
	m.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		options := sheetOptions{
			Title:    L("Working Folder"),
			Subtitle: L("Where %@ mainly works on %@. Its files and changes show beside the chat.", bot.Name, runner),
			Width:    480,
			Confirm:  L("Save"),
		}
		if here {
			options.Leading = func() {
				if pushButton(c.Key("choose"), L("Choose…"), pushOptions{}).Clicked() {
					chooseFolder(m.win, func(path string) {
						if path != "" {
							folder = path
							m.invalidate()
						}
					})
				}
			}
		}
		result := sheetFrame(c, options, func() {
			textField(c, &folder, fieldOptions{AutoFocus: true, Mono: true, Placeholder: L("Its own workspace"), Label: L("Working Folder")})
			ui.Text(c, L("An absolute path or one starting with ~. Leave it empty for its own workspace in Lorca's folder.")).FontSize(11).LineHeight(1.35).TextColor(p.Label2)
		})
		switch {
		case result.Confirmed:
			if current := store.Bot(botID); current != nil && strings.TrimSpace(folder) != current.Workdir {
				// The tree and changes are the new folder's from here on.
				store.SetBotWorkdir(botID, strings.TrimSpace(folder), func(err error) {
					if err != nil {
						m.showAlert(alertOptions{Message: L("Couldn't change the working folder"), Informative: model.ErrorText(err)}, nil)
						return
					}
					delete(m.inspector.workdir, botID)
					m.inspector.refreshWorkdir(botID)
				})
			}
			s.dismiss()
		case result.Cancelled:
			s.dismiss()
		}
	}, nil)
}

// chooseFolder asks for a folder on this computer and answers on the main thread: empty when the
// user cancels.
func chooseFolder(parent *mygo.Window, done func(string)) {
	go func() {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{Parent: parent, Directory: true, CreateDirectories: true, ButtonLabel: L("Choose")})
		path := ""
		if err == nil && len(paths) > 0 {
			path = paths[0]
		}
		post(func() { done(path) })
	}()
}

// presentWorkdirDiff shows one changed file's diff since the last commit, as the bot's Runner has
// it now: hunks with their line numbers, what goes on red and what comes in on green.
func (m *mainWindow) presentWorkdirDiff(botID string, change model.WorkdirChange) {
	var diff *model.WorkdirDiff
	failure := ""
	closed := false
	store.WorkdirDiff(botID, change.Path, func(answer model.WorkdirDiff, err error) {
		if closed {
			return
		}
		if err != nil {
			failure = model.ErrorText(err)
		} else {
			diff = &answer
		}
		m.invalidate()
	})
	added, removed := fmt.Sprintf("+%d", change.Added), fmt.Sprintf("−%d", change.Removed)
	m.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		result := sheetFrame(c, sheetOptions{Title: change.Name(), Subtitle: change.Path + "  " + added + " " + removed, Width: 640, Confirm: L("Done"), NoCancel: true}, func() {
			switch {
			case failure != "":
				ui.Text(c, failure).FontSize(12).TextColor(p.Label2)
			case diff == nil:
				ui.Text(c, L("Loading…")).FontSize(12).TextColor(p.Label2)
			default:
				patchView(c, model.PatchLines(diff.Diff))
				if diff.Truncated {
					ui.Text(c, L("The rest of this diff is too long to show.")).FontSize(11).TextColor(p.Label2)
				}
			}
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, func() { closed = true })
}

// patchView is a file's diff as lines: each hunk's header, the lines around a change, and the
// lines taken out on red and put in on green, with their numbers. Long diffs scroll.
func patchView(c *ui.Context, lines []model.PatchLine) {
	p := colors(c)
	if len(lines) == 0 {
		ui.Text(c, L("No changes to show.")).FontSize(12).TextColor(p.Label2)
		return
	}
	number := func(n int) string {
		if n == 0 {
			return ""
		}
		return fmt.Sprint(n)
	}
	ui.Scroll(c).MaxHeight(420).Radius(8).Border(1, p.Separator).Clip().Children(func() {
		ui.Column(c).Children(func() {
			for i, line := range lines {
				row := ui.Row(c.Key(i)).Gap(8).Padding(1, 10).AlignItems(ui.Start)
				mark, tint, text := " ", p.Label, line.Text
				switch line.Kind {
				case model.PatchAdded:
					mark, tint = "+", p.Green
					row.Background(p.Green.Alpha(0.13))
				case model.PatchRemoved:
					mark, tint = "−", p.Red
					row.Background(p.Red.Alpha(0.13))
				case model.PatchHunk:
					row.Background(p.Accent.Alpha(0.08)).Padding(3, 10)
				}
				row.Children(func() {
					switch line.Kind {
					case model.PatchHunk, model.PatchNote:
						ui.Text(c, text).Grow(1).Shrink(1).MinWidth(0).Font(monoFont).FontSize(11).TextColor(p.Label2).Selectable()
						return
					}
					ui.Text(c, number(line.Old)).Width(30).Shrink(0).TextAlign(ui.End).Font(monoFont).FontSize(10.5).TextColor(p.Label3)
					ui.Text(c, number(line.New)).Width(30).Shrink(0).TextAlign(ui.End).Font(monoFont).FontSize(10.5).TextColor(p.Label3)
					ui.Text(c, mark).Width(10).Shrink(0).Font(monoFont).FontSize(11).FontWeight(600).TextColor(tint)
					if text == "" {
						text = " "
					}
					ui.Text(c, text).Grow(1).Shrink(1).MinWidth(0).Font(monoFont).FontSize(11).LineHeight(1.45).Selectable()
				})
			}
		})
	})
}
