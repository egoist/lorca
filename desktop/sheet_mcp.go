package main

import (
	"regexp"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Adds or edits one of a Runner's MCP servers, after the macOS app's McpServerViewController: its
// name, the command that starts it or its URL, the environment or headers, and what it is for, in a
// form or as the JSON of its mcp.json entry; JSON pasted into a field of the form, as READMEs and
// other apps' configs write it, opens the JSON view. Once saved, the sheet follows the server: how
// it stands, Reconnect or Sign In, a switch to turn it off, and the tools it offered.

// mcpFetching is a server being fetched before its sheet opens, so a second click waits.
var mcpFetching bool

// presentMcpServer is the sheet of one of a Runner's mcp.json servers, fetched first with its
// tools so the sheet opens filled in; an empty name adds one.
func (w *appWindow) presentMcpServer(runner *model.Device, name string) {
	if mcpFetching {
		return
	}
	if name == "" {
		w.presentMcpSheet(runner, nil)
		return
	}
	mcpFetching = true
	store.McpServer(name, runner.ID, func(server model.McpServer, err error) {
		mcpFetching = false
		if err != nil {
			w.showAlert(alertOptions{Message: L("Couldn't open %@", name), Informative: model.ErrorText(err)}, nil)
			return
		}
		w.presentMcpSheet(runner, &server)
	})
}

func (w *appWindow) presentMcpSheet(runner *model.Device, server *model.McpServer) {
	s := &mcpSheet{w: w, runner: runner, saved: server, revealed: map[string]map[int]bool{}}
	entry := model.McpEntry{}
	if server != nil {
		s.name, entry = server.Name, server.Entry
	}
	s.form = model.FormOf(entry)
	// The Runner's state moved: a sign-in finished, a connection came up or failed.
	stop := pluginWatchStore(func(event model.Event) {
		if (event.Kind == model.EventRosterChanged || event.Kind == model.EventSnapshotReplaced) && !s.connecting {
			s.reload()
		}
	})
	w.present(s.view, func() {
		s.closed = true
		stop()
	})
}

// mcpReading is what pasted JSON came to: nothing yet, being read, its servers, or why it could
// not be read.
type mcpReading int

const (
	mcpReadingEmpty mcpReading = iota
	mcpReadingBusy
	mcpReadingDone
	mcpReadingFailed
)

// mcpSheet is the server sheet's state.
type mcpSheet struct {
	w      *appWindow
	runner *model.Device
	// saved is the server as the Runner last answered for it; nil while it is being added.
	saved *model.McpServer

	name   string
	asJSON bool
	form   model.McpForm
	json   string

	reading   mcpReading
	parsed    []model.ParsedServer
	readError string
	// readAt is when the JSON is read, once it stops changing; zero for no read waiting.
	readAt   time.Time
	readText string
	reads    int

	busy, connecting, closed bool
	status                   *providerStatus
	// loads is the newest fetch of the server: an older one's answer is dropped.
	loads int
	// revealed are the secret values shown, by row, of the environment and the headers.
	revealed map[string]map[int]bool
	// focusPair is the list ("env" or "headers") whose new last row takes the keyboard.
	focusPair string
}

// mcpFormLabels are every label the form shows in one mode or another, which its label column is
// as wide as.
func mcpFormLabels() []string {
	return []string{L("Name"), L("JSON"), L("Type"), L("Command"), L("Environment"), L("URL"), L("Headers"), L("About")}
}

var mcpJSONStart = regexp.MustCompile(`^"[^"]+"\s*:\s*\{`)

// mcpLooksLikeJSON is text that reads as JSON rather than a command, a URL, or a name: what a
// README's snippet is.
func mcpLooksLikeJSON(text string) bool {
	trimmed := strings.TrimSpace(text)
	return utf8.RuneCountInString(trimmed) > 2 && (strings.HasPrefix(trimmed, "{") || mcpJSONStart.MatchString(trimmed))
}

// mcpInserted is the text a change put into a field, between what stayed before and after it.
func mcpInserted(before, after string) string {
	b, a := []rune(before), []rune(after)
	start := 0
	for start < len(b) && start < len(a) && b[start] == a[start] {
		start++
	}
	end := 0
	for end < len(b)-start && end < len(a)-start && b[len(b)-1-end] == a[len(a)-1-end] {
		end++
	}
	return string(a[start : len(a)-end])
}

var mcpNonAlnum = regexp.MustCompile(`[^a-z0-9]+`)

// mcpToolPrefix is the prefix bots call the server's tools by: its name, lowercased, with runs of
// anything else as one underscore.
func mcpToolPrefix(name string) string {
	name = strings.TrimSpace(name)
	if name == "" {
		name = "name"
	}
	prefix := strings.Trim(mcpNonAlnum.ReplaceAllString(strings.ToLower(name), "_"), "_")
	return firstNonEmpty(prefix, "name")
}

func (s *mcpSheet) base() model.McpEntry {
	if s.saved != nil {
		return s.saved.Entry
	}
	return model.McpEntry{}
}

// entry is the entry the form, or the one server of the JSON, describes; nil when there is none.
func (s *mcpSheet) entry() model.McpEntry {
	if !s.asJSON {
		return model.EntryOf(s.form, s.base())
	}
	if s.reading == mcpReadingDone && len(s.parsed) == 1 {
		return s.parsed[0].Entry
	}
	return nil
}

// several are the JSON's servers when it holds more than one, which an Add adds together.
func (s *mcpSheet) several() []model.ParsedServer {
	if s.asJSON && s.saved == nil && s.reading == mcpReadingDone && len(s.parsed) > 1 {
		return s.parsed
	}
	return nil
}

// problem is what stops a save, in words, or "".
func (s *mcpSheet) problem() string {
	if !s.asJSON {
		return model.FormProblem(s.name, s.form)
	}
	switch s.reading {
	case mcpReadingEmpty:
		return L("Paste a server's JSON.")
	case mcpReadingBusy:
		return L("Reading…")
	case mcpReadingFailed:
		return s.readError
	}
	// Several are added together, the ones that can run; the note names the others.
	if all := s.several(); all != nil {
		for _, server := range all {
			if server.Problem == "" {
				return ""
			}
		}
		return all[0].Problem
	}
	if len(s.parsed) == 0 {
		return L("Paste a server's JSON.")
	}
	if len(s.parsed) > 1 {
		return L("That JSON has %d servers. Add them from an empty sheet, or keep one.", len(s.parsed))
	}
	if problem := s.parsed[0].Problem; problem != "" {
		return problem
	}
	if strings.TrimSpace(s.name) == "" {
		return L("Give the server a name.")
	}
	return ""
}

// dirty is whether what the sheet shows differs from what the Runner has.
func (s *mcpSheet) dirty() bool {
	if s.saved == nil {
		return true
	}
	if strings.TrimSpace(s.name) != s.saved.Name {
		return true
	}
	next := s.entry()
	return next != nil && !model.SameEntry(next, s.saved.Entry)
}

func (s *mcpSheet) confirmTitle() string {
	switch {
	case s.several() != nil:
		return L("Add %d Servers", len(s.several()))
	case s.saved == nil:
		return L("Add")
	case s.dirty():
		return L("Save")
	}
	return L("Done")
}

func (s *mcpSheet) confirmDisabled() bool {
	return s.busy || (s.dirty() && s.problem() != "")
}

// MARK: - The JSON

// read reads the JSON `delay` from now, once it stops changing, through the CLI here, which knows
// every app's spelling.
func (s *mcpSheet) read(delay time.Duration, text string) {
	s.reads++
	s.readAt = time.Time{}
	if strings.TrimSpace(text) == "" {
		s.reading = mcpReadingEmpty
		return
	}
	s.reading = mcpReadingBusy
	s.readText = text
	if delay == 0 {
		s.startRead()
		return
	}
	s.readAt = time.Now().Add(delay)
}

func (s *mcpSheet) startRead() {
	current := s.reads
	store.ParseMcpJSON(s.readText, func(servers []model.ParsedServer, err error) {
		if s.closed || current != s.reads {
			return
		}
		if err != nil {
			s.reading, s.readError = mcpReadingFailed, model.ErrorText(err)
			return
		}
		s.reading, s.parsed = mcpReadingDone, servers
		// One named server: its name fills an empty Name.
		if len(servers) == 1 && servers[0].Name != "" && strings.TrimSpace(s.name) == "" {
			s.name = servers[0].Name
		}
	})
}

// showJSON opens the JSON view with pasted text, or with the form's entry when pasted is "".
func (s *mcpSheet) showJSON(pasted string) {
	text := pasted
	if text == "" {
		text = model.EntryJSON(model.EntryOf(s.form, s.base()))
	}
	s.json = text
	s.asJSON = true
	s.read(0, text)
}

// showForm goes back to the form, which the JSON's one server fills.
func (s *mcpSheet) showForm() {
	text := s.json
	if strings.TrimSpace(text) == "" {
		s.asJSON = false
		return
	}
	store.ParseMcpJSON(text, func(servers []model.ParsedServer, err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
			return
		}
		if len(servers) != 1 {
			s.status = &providerStatus{text: L("The form holds one server, and that JSON has %d.", len(servers)), tone: model.ToneOrange}
			return
		}
		server := servers[0]
		if server.Entry == nil {
			s.status = &providerStatus{text: firstNonEmpty(server.Problem, L("That JSON is not a server.")), tone: model.ToneRed}
			return
		}
		s.form = model.FormOf(server.Entry)
		if server.Name != "" && strings.TrimSpace(s.name) == "" {
			s.name = server.Name
		}
		s.status = nil
		s.asJSON = false
	})
}

// pasted takes JSON pasted into a field of the form to the JSON view: the field keeps what it had.
func (s *mcpSheet) pasted(value *string, before string) {
	inserted := mcpInserted(before, *value)
	if utf8.RuneCountInString(inserted) < 2 || !mcpLooksLikeJSON(inserted) {
		return
	}
	*value = before
	s.showJSON(inserted)
}

// MARK: - The server

// reload fetches the server as the Runner has it now: its state and its tools.
func (s *mcpSheet) reload() {
	current := s.saved
	if current == nil {
		return
	}
	s.loads++
	load := s.loads
	store.McpServer(current.Name, s.runner.ID, func(fresh model.McpServer, err error) {
		// The list says when the Runner cannot be reached; the sheet keeps what it has.
		if err != nil || s.closed || load != s.loads {
			return
		}
		s.saved = &fresh
	})
}

// connect connects the server and waits for how it went: from scratch for Reconnect, or taking the
// connection a save started.
func (s *mcpSheet) connect(fresh bool, server *model.McpServer) {
	if server == nil {
		return
	}
	s.connecting = true
	s.loads++
	store.ReconnectMcpServer(s.runner.ID, server.Name, fresh, func(answered model.McpServer, err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
		} else {
			s.saved = &answered
		}
		s.connecting = false
	})
}

func (s *mcpSheet) signIn() {
	current := s.saved
	if current == nil {
		return
	}
	// The Runner notes the sign-in on the plugin, so the state reads it.
	store.ConnectPlugin(current.ID, s.runner.ID, func(err error) {
		if err != nil {
			s.w.showAlert(alertOptions{Message: L("Couldn't start the sign-in"), Informative: model.ErrorText(err)}, nil)
			return
		}
		s.reload()
	})
}

// showTool offers a tool to bots or keeps it from them; the switch moves at once, and back on a
// failure.
func (s *mcpSheet) showTool(tool string, shown bool) {
	current := s.saved
	if current == nil {
		return
	}
	next := *current
	next.Tools = slices.Clone(current.Tools)
	for i := range next.Tools {
		if next.Tools[i].Name == tool {
			next.Tools[i].Hidden = !shown
		}
	}
	s.saved = &next
	store.SetMcpToolHidden(s.runner.ID, current.Name, tool, !shown, func(answered model.McpServer, err error) {
		if err != nil {
			if !s.closed {
				s.saved = current
			}
			message := L("Couldn't hide %@", tool)
			if shown {
				message = L("Couldn't offer %@ to bots", tool)
			}
			s.w.showAlert(alertOptions{Message: message, Informative: model.ErrorText(err)}, nil)
			return
		}
		if !s.closed {
			s.saved = &answered
		}
	})
}

// signOut forgets the sign-in on the Runner. Nothing is revoked at the server; its next use asks
// again.
func (s *mcpSheet) signOut() {
	current := s.saved
	if current == nil {
		return
	}
	store.SignOutMcpServer(s.runner.ID, current.Name, func(answered model.McpServer, err error) {
		if err != nil {
			s.w.showAlert(alertOptions{Message: L("Couldn't sign out of %@", current.Name), Informative: model.ErrorText(err)}, nil)
			return
		}
		if !s.closed {
			s.saved = &answered
		}
	})
}

func (s *mcpSheet) setEnabled(on bool) {
	current := s.saved
	if current == nil {
		return
	}
	next := *current
	next.Enabled = on
	s.saved = &next
	store.SetMcpServerEnabled(s.runner.ID, current.Name, on, func(answered model.McpServer, err error) {
		if err != nil {
			if !s.closed {
				s.saved = current
			}
			message := L("Couldn't turn %@ off", current.Name)
			if on {
				message = L("Couldn't turn %@ on", current.Name)
			}
			s.w.showAlert(alertOptions{Message: message, Informative: model.ErrorText(err)}, nil)
			return
		}
		if s.closed {
			return
		}
		s.saved = &answered
		if on {
			s.connect(false, &answered)
		}
	})
}

// MARK: - Saving

func (s *mcpSheet) begin(text string) {
	s.busy = true
	s.status = &providerStatus{text: text, tone: model.ToneSecondary, spinning: true}
}

func (s *mcpSheet) fail(err error) {
	s.busy = false
	s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
}

func (s *mcpSheet) confirm(sh *sheet) {
	if s.confirmDisabled() {
		return
	}
	if !s.dirty() {
		sh.dismiss()
		return
	}
	if servers := s.several(); servers != nil {
		s.addAll(sh, servers)
		return
	}
	next := s.entry()
	if next == nil {
		return
	}
	name := strings.TrimSpace(s.name)
	previous := ""
	if s.saved != nil {
		previous = s.saved.Name
		s.begin(L("Saving %@…", name))
	} else {
		s.begin(L("Adding %@…", name))
	}
	store.SaveMcpServer(s.runner.ID, name, next, previous, func(server model.McpServer, err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.fail(err)
			return
		}
		s.saved = &server
		s.name = server.Name
		s.form = model.FormOf(server.Entry)
		s.asJSON = false
		s.busy = false
		s.status = nil
		// The Runner starts it once to list its tools; the sheet waits on that connection.
		if server.Enabled {
			s.connect(false, &server)
		}
	})
}

// addAll adds the servers of pasted JSON that can run, one after another, and says which could not
// be added.
func (s *mcpSheet) addAll(sh *sheet, servers []model.ParsedServer) {
	var usable []model.ParsedServer
	var failed []string
	for _, server := range servers {
		if server.Name != "" && server.Entry != nil && server.Problem == "" {
			usable = append(usable, server)
		} else {
			failed = append(failed, firstNonEmpty(server.Name, "?")+": "+firstNonEmpty(server.Problem, L("That JSON is not a server.")))
		}
	}
	s.begin(L("Adding %d servers…", len(usable)))
	var add func(i int)
	add = func(i int) {
		if s.closed {
			return
		}
		if i == len(usable) {
			s.busy = false
			if len(failed) == 0 {
				sh.dismiss()
			} else {
				s.status = &providerStatus{text: strings.Join(failed, "\n"), tone: model.ToneRed}
			}
			return
		}
		server := usable[i]
		store.SaveMcpServer(s.runner.ID, server.Name, server.Entry, "", func(_ model.McpServer, err error) {
			if err != nil {
				failed = append(failed, server.Name+": "+model.ErrorText(err))
			}
			add(i + 1)
		})
	}
	add(0)
}

func (s *mcpSheet) confirmRemove(sh *sheet) {
	current := s.saved
	if current == nil {
		return
	}
	s.w.showAlert(alertOptions{
		Message:     L("Remove %@ from %@?", current.Name, s.runner.Name),
		Informative: L("Its entry leaves mcp.json on %@, every bot there loses its tools, and its sign-in is forgotten.", s.runner.Name),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Remove")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 || s.closed {
			return
		}
		s.begin(L("Removing…"))
		store.RemoveMcpServer(s.runner.ID, current.Name, func(err error) {
			if s.closed {
				return
			}
			if err != nil {
				s.fail(err)
				return
			}
			sh.dismiss()
		})
	})
}

// MARK: - The view

func (s *mcpSheet) view(c *ui.Context, sh *sheet) {
	if !s.readAt.IsZero() {
		if now := c.Now(); now.Before(s.readAt) {
			c.After(s.readAt.Sub(now))
		} else {
			s.readAt = time.Time{}
			s.startRead()
		}
	}
	title := L("Add MCP Server")
	var leading func()
	if s.saved != nil {
		title = s.saved.Name
		leading = func() {
			if pushButton(c, L("Remove…"), pushOptions{Kind: buttonDestructive, Disabled: s.busy}).Clicked() {
				s.confirmRemove(sh)
			}
		}
	}
	result := sheetFrame(c, sheetOptions{
		Title:           title,
		Subtitle:        L("A command %@ runs, or a remote server's URL, speaking the Model Context Protocol. Every bot on %@ can use its tools.", s.runner.Name, s.runner.Name),
		Width:           560,
		Confirm:         s.confirmTitle(),
		ConfirmDisabled: s.confirmDisabled(),
		Leading:         leading,
	}, func() {
		if s.saved != nil {
			s.statusView(c)
		}
		mode := 0
		if s.asJSON {
			mode = 1
		}
		ui.Row(c).Justify(ui.End).Margin(0, 0, -4, 0).Children(func() {
			segmented(c, &mode, L("Edit as"), L("Form"), L("JSON")).OnChange(func() {
				if mode == 1 {
					s.showJSON("")
				} else {
					s.showForm()
				}
			})
		})
		s.formView(c)
		if !s.asJSON && s.saved == nil {
			providerNote(c, L("Paste the JSON a server's README gives, or a whole mcpServers block from Claude Desktop or Cursor, into any field."), nil)
		}
		providerStatusLine(c, s.status)
	})
	if result.Confirmed {
		s.confirm(sh)
	}
	if result.Cancelled {
		sh.dismiss()
	}
}

// formView is the server's name and, in the form, its command or URL with the environment or
// headers, and what it is for; or its JSON. The labels sit in a column as wide as the widest the
// form shows in any mode, so switching between Command and URL, or Form and JSON, moves no field.
func (s *mcpSheet) formView(c *ui.Context) {
	p := colors(c)
	labelWidth := mcpFormLabelWidth(c, mcpFormLabels()...)
	several := s.several()
	ui.Column(c).Gap(8).Children(func() {
		if several == nil {
			providerFormRow(c, labelWidth, L("Name"), func() {
				s.pasteField(c, &s.name, fieldOptions{Placeholder: "github", Disabled: s.busy, AutoFocus: true, Label: L("Name")})
			})
			providerFormNote(c, labelWidth, L("Bots call its tools as %@__tool.", mcpToolPrefix(s.name)), nil)
		}
		if s.asJSON {
			providerFormRow(c, labelWidth, L("JSON"), func() {
				area := textArea(c, &s.json, 0, fieldOptions{
					Mono:        true,
					Disabled:    s.busy,
					Label:       L("The server's JSON"),
					Placeholder: "{\n  \"command\": \"npx\",\n  \"args\": [\"-y\", \"@modelcontextprotocol/server-memory\"]\n}",
				}).Height(196).FontSize(11.5)
				if area.Changed() {
					s.read(300*time.Millisecond, s.json)
				}
			})
			var tint *ui.Color
			if s.reading == mcpReadingFailed {
				tint = &p.Red
			}
			providerFormNote(c, labelWidth, s.jsonNote(several), tint)
			return
		}
		kind := 0
		if s.form.Remote {
			kind = 1
		}
		providerFormRow(c, labelWidth, L("Type"), func() {
			ui.Row(c).Children(func() {
				segmented(c, &kind, L("Type"), L("Command"), L("URL")).OnChange(func() {
					s.form.Remote = kind == 1
				})
			})
		})
		if s.form.Remote {
			providerFormRow(c, labelWidth, L("URL"), func() {
				s.pasteField(c, &s.form.URL, fieldOptions{Placeholder: "https://mcp.example.com/mcp", Mono: true, Disabled: s.busy, Label: L("URL")})
			})
			providerFormNote(c, labelWidth, L("Streamable HTTP. When the server asks for a sign-in, Lorca signs in with OAuth; or send a token in a header."), nil)
			s.pairsRow(c, labelWidth, L("Headers"), "headers", &s.form.Headers, "Authorization", "Bearer …", L("Add Header"))
		} else {
			providerFormRow(c, labelWidth, L("Command"), func() {
				s.pasteField(c, &s.form.Command, fieldOptions{Placeholder: "npx -y @modelcontextprotocol/server-filesystem ~/Documents", Mono: true, Disabled: s.busy, Label: L("Command")})
			})
			note := L("Starts on %@ from its login shell, so npx, uvx, and docker resolve as they do in a terminal there.", s.runner.Name)
			if s.runner.IsThisDevice {
				note = L("Starts on this computer from your login shell, so npx, uvx, and docker resolve as they do in a terminal.")
			}
			providerFormNote(c, labelWidth, note, nil)
			s.pairsRow(c, labelWidth, L("Environment"), "env", &s.form.Env, "API_KEY", L("value"), L("Add Variable"))
		}
		providerFormRow(c, labelWidth, L("About"), func() {
			textField(c, &s.form.Description, fieldOptions{Placeholder: L("What it is for, which bots read (optional)"), Disabled: s.busy, Label: L("About")})
		})
	})
}

// pasteField keeps the value preceding the edit until the next build observes Changed.
func (s *mcpSheet) pasteField(c *ui.Context, value *string, options fieldOptions) {
	field := textField(c, value, options)
	before := ui.Local(field, "before", func() string { return *value })
	if field.Changed() {
		s.pasted(value, *before)
	}
	*before = *value
}

// jsonNote is what the JSON holds, or why it cannot be read.
func (s *mcpSheet) jsonNote(several []model.ParsedServer) string {
	switch s.reading {
	case mcpReadingEmpty:
		return L("One server's JSON, or the mcpServers block of Claude Desktop, Cursor, or a README.")
	case mcpReadingBusy:
		return L("Reading…")
	case mcpReadingFailed:
		return s.readError
	}
	if several != nil {
		names := make([]string, 0, len(several))
		for _, server := range several {
			names = append(names, firstNonEmpty(server.Name, "?"))
		}
		return L("%d servers: %@", len(several), strings.Join(names, ", "))
	}
	if len(s.parsed) == 0 {
		return ""
	}
	server := s.parsed[0]
	if server.Problem != "" {
		return server.Problem
	}
	if server.Entry == nil {
		return ""
	}
	if server.Entry.IsRemote() {
		return L("A remote server at %@", model.McpAddress(server.Entry))
	}
	return L("A command: %@", model.McpAddress(server.Entry))
}

// pairsRow is the environment's or the headers' row of the form: its label sits on the first
// row's text, the first row's fields or the Add link when there are none.
func (s *mcpSheet) pairsRow(c *ui.Context, labelWidth float32, label, list string, pairs *[]model.Pair, namePlaceholder, valuePlaceholder, addTitle string) {
	p := colors(c)
	ui.Row(c).Gap(10).AlignItems(ui.Start).Children(func() {
		text := ui.Text(c, label).Width(labelWidth).Shrink(0).FontSize(12).TextColor(p.Label2).SingleLine()
		if len(*pairs) > 0 {
			text.Padding(6, 0, 0, 0)
		}
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
			s.pairsEditor(c, label, list, pairs, namePlaceholder, valuePlaceholder, addTitle)
		})
	})
}

// pairsEditor is names and values as rows of fields, a link to add a row, and a button to remove
// each: a command's environment, or a remote server's headers. A value whose name says it is a key
// is hidden until its eye shows it.
func (s *mcpSheet) pairsEditor(c *ui.Context, label, list string, pairs *[]model.Pair, namePlaceholder, valuePlaceholder, addTitle string) {
	p := colors(c)
	revealed := s.revealed[list]
	if revealed == nil {
		revealed = map[int]bool{}
		s.revealed[list] = revealed
	}
	removed, added := -1, false
	ui.Column(c).Gap(6).Label(label).Children(func() {
		for i := range *pairs {
			pair := &(*pairs)[i]
			ui.Row(c.Key(i)).Gap(6).Children(func() {
				name := textField(c, &pair.Name, fieldOptions{Placeholder: namePlaceholder, Mono: true, Disabled: s.busy, Label: L("Name")}).
					MinHeight(24).Padding(2, 7).Grow(1).Basis(0).MinWidth(0)
				if s.focusPair == list && i == len(*pairs)-1 {
					s.focusPair = ""
					name.Focus()
				}
				secret := model.LooksSecret(pair.Name)
				shown := revealed[i]
				ui.Box(c).Grow(1.35).Basis(0).MinWidth(0).Children(func() {
					value := textField(c, &pair.Value, fieldOptions{Placeholder: valuePlaceholder, Mono: true, Secure: secret && !shown, Disabled: s.busy, Label: L("Value")}).
						MinHeight(24).Padding(2, 7)
					if !secret {
						return
					}
					value.Padding(2, 28, 2, 7)
					symbolName, tooltip := "eye", L("Show value")
					if shown {
						symbolName, tooltip = "eye.slash", L("Hide value")
					}
					eye := hoverButton(c, hoverButtonOptions{Symbol: symbolName, Size: 13, Tooltip: tooltip, Disabled: s.busy}).
						Size(24, 20).MinWidth(24).Attach(ui.AnchorRight, ui.AnchorRight).Right(3)
					if eye.Clicked() {
						revealed[i] = !shown
					}
				})
				if hoverButton(c, hoverButtonOptions{Symbol: "minus.circle", Size: 14, Tooltip: L("Remove"), Disabled: s.busy}).Clicked() {
					removed = i
				}
			})
		}
		add := ui.ButtonBase(c).AlignSelf(ui.Start).Gap(4).FontSize(12).FontWeight(500).TextColor(p.Accent).Label(addTitle).Cursor(ui.CursorPointer)
		if s.busy {
			add.Disabled(true)
		}
		add.Children(func() {
			symbol(c, "plus", 11, 2.4)
			ui.Text(c, addTitle).SingleLine()
		})
		added = add.Clicked()
	})
	switch {
	case removed >= 0:
		*pairs = slices.Delete(slices.Clone(*pairs), removed, removed+1)
		next := map[int]bool{}
		for at, on := range revealed {
			switch {
			case at < removed:
				next[at] = on
			case at > removed:
				next[at-1] = on
			}
		}
		s.revealed[list] = next
	case added:
		// The new row's name takes the keyboard.
		*pairs = append(slices.Clone(*pairs), model.Pair{})
		s.focusPair = list
	}
}

// statusView is how a saved server stands, with what to do about it, whether it is on, and its
// tools.
func (s *mcpSheet) statusView(c *ui.Context) {
	p := colors(c)
	server := *s.saved
	text, tone := model.McpStateText(server)
	color := p.tone(tone)
	if s.connecting {
		text, color = L("Connecting…"), p.Accent
	}
	needsSignIn := !s.connecting && server.Status != nil && (server.Status.State == model.PluginNeedsAuth || server.Status.State == model.PluginInsufficientAccess)
	section(c, L("Status"), sectionCaption, nil, func(k *card) {
		state := k.row(rowBox(c).MinHeight(36).Label(L("State")))
		state.Children(func() {
			rowKey(c, L("State"))
			ui.Row(c).Grow(1).Shrink(1).MinWidth(0).Gap(6).Justify(ui.End).Children(func() {
				if s.connecting {
					spinner(c, 12)
				} else {
					ui.Box(c).Size(7, 7).Radius(3.5).Shrink(0).Background(color)
				}
				ui.Text(c, text).Shrink(1).MinWidth(0).FontSize(12).TextAlign(ui.End).TextColor(color).Selectable()
			})
			if server.Enabled && server.Problem == "" {
				title := L("Reconnect")
				if needsSignIn {
					title = L("Sign in")
				}
				if linkButton(c, title, s.connecting || s.busy).Clicked() {
					if needsSignIn {
						s.signIn()
					} else {
						s.connect(true, s.saved)
					}
				}
			}
		})
		if server.SignsIn && server.SignedIn && !needsSignIn {
			account := k.row(rowBox(c).Label(L("Account")))
			account.Children(func() {
				rowKey(c, L("Account"))
				ui.Text(c, L("Signed in")).Grow(1).Shrink(1).MinWidth(0).FontSize(12).TextAlign(ui.End).TextColor(p.Green).SingleLine()
				if linkButton(c, L("Sign in again"), s.busy).Clicked() {
					s.signIn()
				}
				if linkButton(c, L("Sign Out"), s.busy).Clicked() {
					s.signOut()
				}
			})
		}
		if server.Problem == "" {
			accessoryRow(c, k, L("On"), L("Off, no bot on %@ sees it and it never starts.", s.runner.Name), func() {
				on := server.Enabled
				if toggleSwitch(c, &on, true).Label(L("On")).Disabled(s.busy).Changed() {
					s.setEnabled(on)
				}
			})
		}
	})
	if len(server.Tools) == 0 {
		return
	}
	section(c, L("Tools"), sectionCaption, nil, func(k *card) {
		ui.Scroll(c).MaxHeight(172).Children(func() {
			rows := &card{line: p.Separator}
			for _, tool := range server.Tools {
				r := rows.row(ui.Row(c.Key(tool.Name)).Gap(8).MinWidth(0).Padding(5, 12))
				if tool.Description != "" {
					r.Tooltip(tool.Description)
				}
				// A tool kept from bots reads dimmed beside its switch.
				dim := float32(1)
				if tool.Hidden {
					dim = 0.45
				}
				r.Children(func() {
					ui.Text(c, tool.Name).Shrink(0).MaxWidthPercent(45).Font(monoFont).FontSize(11.5).FontWeight(600).SingleLine().Opacity(dim)
					ui.Text(c, tool.Description).Grow(1).Shrink(1).MinWidth(0).FontSize(textCaption).TextColor(p.Label2).SingleLine().Opacity(dim)
					if tool.ReadOnly {
						ui.Row(c).Shrink(0).Height(16).Padding(0, 6).Radius(8).Background(p.Chip).Opacity(dim).
							Tooltip(L("It only reads, so it runs without Auto-review.")).Children(func() {
							ui.Text(c, L("Reads only")).FontSize(10.5).TextColor(p.Label2).SingleLine()
						})
					}
					on := !tool.Hidden
					tooltip := L("Offered to bots")
					if tool.Hidden {
						tooltip = L("Hidden from bots")
					}
					toggleSwitch(c, &on, true).Label(L("Offer %@ to bots", tool.Name)).Tooltip(tooltip).Disabled(s.busy).
						OnChange(func() { s.showTool(tool.Name, on) })
				})
			}
		})
	})
}
