package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// sheetATester is the demo's main window with a sheet put up by `open`, its store's answers in.
func sheetATester(t *testing.T, open func(m *mainWindow)) (*mainWindow, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	tt.Frame()
	open(m)
	sheetASettle(tt)
	return m, tt
}

// sheetASettle runs the store's answers and the frames they bring.
func sheetASettle(tt *ui.Tester) {
	for range 3 {
		runPosts()
		tt.Frame()
	}
}

// sheetAWait waits out the demo store's timers (a listing, a reconnect), then settles.
func sheetAWait(tt *ui.Tester, d time.Duration) {
	time.Sleep(d)
	sheetASettle(tt)
}

func sheetARender(t *testing.T, tt *ui.Tester, name string) {
	t.Helper()
	tt.SetDark(false)
	tt.Frame()
	renderTo(t, tt, name+"-light")
	tt.SetDark(true)
	tt.Frame()
	renderTo(t, tt, name+"-dark")
	tt.SetDark(false)
	tt.Frame()
}

func sheetAWorkbench() *model.Device { return store.Device("dev-workbench") }

func sheetAServer(t *testing.T, name string) model.McpServer {
	t.Helper()
	var found model.McpServer
	ok := false
	store.McpServer(name, "dev-workbench", func(server model.McpServer, err error) { found, ok = server, err == nil })
	runPosts()
	if !ok {
		t.Fatalf("no server %s", name)
	}
	return found
}

func TestRenderMcpServerSheet(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) { m.presentMcpServer(sheetAWorkbench(), "filesystem") })
	for _, text := range []string{"filesystem", "STATUS", "4 tools", "Reconnect", "TOOLS", "read_text_file", "Reads only", "Environment", "Add Variable", "Done", "Remove…"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "mcp-filesystem")

	// JSON shows the entry as mcp.json has it.
	if err := tt.Click("JSON"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText("A command: npx -y @modelcontextprotocol/server-filesystem ~/Documents/Notes") {
		t.Errorf("JSON note: %q", tt.Texts())
	}
	sheetARender(t, tt, "mcp-filesystem-json")
	if err := tt.Click("Form"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText("Command") || !tt.HasText("Done") {
		t.Errorf("back to the form: %q", tt.Texts())
	}

	// A tool's switch hides it from bots in mcp.json.
	if err := tt.Click("Offer write_file to bots"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	for _, tool := range sheetAServer(t, "filesystem").Tools {
		if tool.Name == "write_file" && !tool.Hidden {
			t.Errorf("write_file still offered")
		}
	}
}

func TestRenderMcpServerSheetStates(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) { m.presentMcpServer(sheetAWorkbench(), "sentry") })
	if !tt.HasText("Needs a sign-in") || !tt.HasText("Sign in") || !tt.HasText("Headers") {
		t.Errorf("sentry: %q", tt.Texts())
	}
	sheetARender(t, tt, "mcp-sentry")

	_, tt = sheetATester(t, func(m *mainWindow) { m.presentMcpServer(sheetAWorkbench(), "postgres") })
	if !tt.HasText("Off") || tt.HasText("Reconnect") || !tt.HasText("Environment") {
		t.Errorf("postgres: %q", tt.Texts())
	}
	sheetARender(t, tt, "mcp-postgres")

	// Turning it on connects it.
	sheetAClickEnd(t, tt, "On")
	sheetASettle(tt)
	if !sheetAServer(t, "postgres").Enabled {
		t.Errorf("postgres still off")
	}
	if !tt.HasText("Connecting…") {
		t.Errorf("not connecting: %q", tt.Texts())
	}
	sheetAWait(tt, time.Second)
	// The demo's server never listed its tools.
	if tt.HasText("Connecting…") || !tt.HasText("Not connected yet") || !tt.HasText("Reconnect") {
		t.Errorf("after connecting: %q", tt.Texts())
	}
}

func TestMcpServerSheetKeepsJSONDraft(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) { m.presentMcpServer(sheetAWorkbench(), "filesystem") })
	if err := tt.Click("JSON"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if err := tt.Click("The server's JSON"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type(`{"command":"edited-server"}`)
	sheetAWait(tt, 400*time.Millisecond)
	if !tt.HasText("A command: edited-server") || !tt.HasText("Save") {
		t.Fatal("the JSON edit was not read")
	}
	// Selecting the current mode again keeps the draft rather than rebuilding it from the form.
	if err := tt.Click("JSON"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText("A command: edited-server") || !tt.HasText("Save") {
		t.Fatal("selecting JSON again replaced the draft")
	}
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if got := sheetAServer(t, "filesystem").Entry.Command(); got != "edited-server" {
		t.Fatalf("saved command %q", got)
	}
}

func TestMcpServerSheetAdds(t *testing.T) {
	m, tt := sheetATester(t, func(m *mainWindow) { m.presentMcpServer(sheetAWorkbench(), "") })
	for _, text := range []string{"Add MCP Server", "Name", "Type", "Command", "Environment", "About", "Add"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "mcp-add")
	if !tt.Focused("Name") {
		t.Errorf("Name has no focus")
	}
	tt.Type("Memory Box")
	tt.Frame()
	if !tt.HasText("Bots call its tools as memory_box__tool.") {
		t.Errorf("tool prefix: %q", tt.Texts())
	}

	// The URL fields, and back.
	if err := tt.Click("URL"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("Headers") || !tt.HasText("Add Header") {
		t.Errorf("URL fields: %q", tt.Texts())
	}
	if err := tt.Click("Command"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	tt.Key(0, ui.KeyRight)
	tt.Frame()
	if !tt.HasText("Headers") {
		t.Fatal("the right arrow did not select URL")
	}
	tt.Key(0, ui.KeyLeft)
	tt.Frame()
	if !tt.HasText("Environment") {
		t.Fatal("the left arrow did not select Command")
	}

	// A variable row, its secret value hidden behind an eye.
	if err := tt.Click("Add Variable"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	tt.Type("API_KEY")
	tt.Frame()
	if _, ok := tt.Find("Show value"); !ok {
		t.Errorf("no eye for a key: %q", tt.Texts())
	}
	sheetARender(t, tt, "mcp-add-env")
	if err := tt.Click("Remove"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()

	// JSON pasted into the Command field opens the JSON view with it.
	sheetAFocusCommand(t, tt)
	tt.SetClipboard(`{"mcpServers": {"memory": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-memory"]}, "time": {"command": "uvx", "args": ["mcp-server-time"]}}}`)
	tt.Key(ui.Cmd, ui.KeyV)
	sheetASettle(tt)
	if !tt.HasText("2 servers: memory, time") || !tt.HasText("Add 2 Servers") {
		t.Fatalf("pasted JSON: %q", tt.Texts())
	}
	sheetARender(t, tt, "mcp-add-several")
	if err := tt.Click("Add 2 Servers"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if m.hasSheet() {
		t.Errorf("sheet still up: %q", tt.Texts())
	}
	names := map[string]bool{}
	store.McpServers("dev-workbench", func(file model.McpFile, _ error) {
		for _, server := range file.Servers {
			names[server.Name] = true
		}
	})
	runPosts()
	if !names["memory"] || !names["time"] {
		t.Errorf("servers %v", names)
	}
}

func TestMcpServerSheetSaves(t *testing.T) {
	m, tt := sheetATester(t, func(m *mainWindow) { m.presentMcpServer(sheetAWorkbench(), "") })
	tt.Type("notes")
	sheetAFocusCommand(t, tt)
	tt.Type("npx -y notes-server")
	tt.Frame()
	if err := tt.Click("Add"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	server := sheetAServer(t, "notes")
	if server.Entry.Command() != "npx" || len(server.Entry.Args()) != 2 {
		t.Errorf("entry %v", server.Entry)
	}
	// Once saved, the sheet follows the server.
	if !tt.HasText("notes") || !tt.HasText("STATUS") || !tt.HasText("Done") {
		t.Errorf("after the save: %q", tt.Texts())
	}
	sheetAWait(tt, time.Second)
	if !m.hasSheet() {
		t.Fatal("sheet gone")
	}
	// Remove asks first.
	if err := tt.Click("Remove…"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("Remove notes from Workbench?") {
		t.Fatalf("no question: %q", tt.Texts())
	}
	if err := tt.Click("Remove"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if m.hasSheet() {
		t.Errorf("sheet still up: %q", tt.Texts())
	}
	store.McpServer("notes", "dev-workbench", func(_ model.McpServer, err error) {
		if err == nil {
			t.Errorf("notes still in mcp.json")
		}
	})
	runPosts()
}

// sheetAFocusCommand clicks into the Command field, just above the note under it.
func sheetAFocusCommand(t *testing.T, tt *ui.Tester) {
	t.Helper()
	note, ok := tt.Find("Starts on this computer from your login shell, so npx, uvx, and docker resolve as they do in a terminal.")
	if !ok {
		t.Fatal("no Command note")
	}
	tt.ClickAt(note.X+40, note.Y-12)
	tt.Frame()
}

// sheetAClickEnd clicks the control at the end of the row `label` names, as its switch.
func sheetAClickEnd(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	r, ok := tt.Find(label)
	if !ok {
		t.Fatalf("no %s", label)
	}
	tt.ClickAt(r.X+r.W-24, r.Y+r.H/2)
	tt.Frame()
}

func TestMcpInsertedAndPrefix(t *testing.T) {
	for _, c := range []struct{ before, after, want string }{
		{"", "{}", "{}"},
		{"npx", "np{\"a\":1}x", "{\"a\":1}"},
		{"abc", "abcd", "d"},
		{"aaa", "aaaa", "a"},
	} {
		if got := mcpInserted(c.before, c.after); got != c.want {
			t.Errorf("mcpInserted(%q, %q) = %q, want %q", c.before, c.after, got, c.want)
		}
	}
	for name, want := range map[string]string{"": "name", "GitHub": "github", "My Server!": "my_server", "--": "name"} {
		if got := mcpToolPrefix(name); got != want {
			t.Errorf("mcpToolPrefix(%q) = %q, want %q", name, got, want)
		}
	}
	if mcpLooksLikeJSON("npx") || !mcpLooksLikeJSON(`"memory": {"command": "npx"}`) || !mcpLooksLikeJSON(`{"url": "x"}`) {
		t.Errorf("mcpLooksLikeJSON")
	}
}
