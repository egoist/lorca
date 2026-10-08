package model

import (
	"reflect"
	"testing"
)

// A Runner's MCP servers in the model: the command line the server sheet reads and writes, the
// form it edits an entry through, and how a server's state reads.

func TestCommandLineSplitsAsWindowsAndAShellRead(t *testing.T) {
	cases := []struct {
		line string
		want []string
	}{
		{"npx -y @modelcontextprotocol/server-filesystem ~/Documents", []string{"npx", "-y", "@modelcontextprotocol/server-filesystem", "~/Documents"}},
		{`"C:\Program Files\nodejs\node.exe" server.js`, []string{`C:\Program Files\nodejs\node.exe`, "server.js"}},
		// A backslash is a backslash, unless a run of them comes before a quote.
		{`C:\Users\me\mcp.exe \\server\share`, []string{`C:\Users\me\mcp.exe`, `\\server\share`}},
		{`"say \"hi\"" 'single quoted' it's`, []string{`say "hi"`, "single quoted", "its"}},
		{`"C:\dir\\" next`, []string{`C:\dir\`, "next"}},
		{`a "" b`, []string{"a", "", "b"}},
		{"  spaced   out  ", []string{"spaced", "out"}},
		{"", nil},
	}
	for _, test := range cases {
		if got := SplitCommandLine(test.line); !reflect.DeepEqual(got, test.want) {
			t.Errorf("SplitCommandLine(%q) = %q, want %q", test.line, got, test.want)
		}
	}
}

func TestJoinedWordsReadBackAsTheSameWords(t *testing.T) {
	cases := [][]string{
		{"npx", "-y", "@scope/pkg"},
		{`C:\Program Files\nodejs\node.exe`, `C:\My Server\index.js`},
		{"echo", `a "quoted" word`, "it's", ""},
		{`trailing\`, `dir with space\`, `\\unc\path`},
		{"docker", "run", "-i", "--rm", "-e", "GITHUB_TOKEN", "ghcr.io/github/github-mcp-server"},
	}
	for _, words := range cases {
		if got := SplitCommandLine(JoinCommandLine(words)); !reflect.DeepEqual(got, words) {
			t.Errorf("%q came back as %q", words, got)
		}
	}
	if got := JoinCommandLine([]string{"npx", "-y", "My Folder"}); got != `npx -y "My Folder"` {
		t.Errorf("got %s", got)
	}
	if got := JoinCommandLine([]string{`C:\Program Files\x.exe`}); got != `"C:\Program Files\x.exe"` {
		t.Errorf("got %s", got)
	}
}

func TestFormEditsAnEntryAndKeepsWhatItDoesNotShow(t *testing.T) {
	entry := McpEntry{"command": "npx", "args": []string{"-y", "pkg", "My Folder"}, "env": map[string]string{"API_KEY": "secret", "DEBUG": "1"}, "cwd": "~/work", "alwaysAllow": []any{"read"}, "disabled": true}
	form := FormOf(entry)
	want := McpForm{Command: `npx -y pkg "My Folder"`, Env: []Pair{{"API_KEY", "secret"}, {"DEBUG", "1"}}, Headers: []Pair{}}
	if !reflect.DeepEqual(form, want) {
		t.Fatalf("form %+v", form)
	}
	// Untouched, it says what the entry said.
	if !SameEntry(EntryOf(form, entry), entry) {
		t.Errorf("untouched form changed the entry: %v", EntryOf(form, entry))
	}
	// A new command and an emptied environment row; cwd, alwaysAllow, and disabled stay.
	edited := form
	edited.Command, edited.Env = "uvx server --port 1", []Pair{{"DEBUG", "2"}, {"", "dropped"}}
	wantEdited := McpEntry{"command": "uvx", "args": []string{"server", "--port", "1"}, "env": map[string]string{"DEBUG": "2"}, "cwd": "~/work", "alwaysAllow": []any{"read"}, "disabled": true}
	if got := EntryOf(edited, entry); !reflect.DeepEqual(got, wantEdited) {
		t.Errorf("edited %v", got)
	}
	// To a URL: the command's fields go, a type comes.
	remote := form
	remote.Remote, remote.URL, remote.Headers = true, " https://mcp.example.com/mcp ", []Pair{{"Authorization", "Bearer x"}}
	wantRemote := McpEntry{"type": "http", "url": "https://mcp.example.com/mcp", "headers": map[string]string{"Authorization": "Bearer x"}, "cwd": "~/work", "alwaysAllow": []any{"read"}, "disabled": true}
	if got := EntryOf(remote, entry); !reflect.DeepEqual(got, wantRemote) {
		t.Errorf("remote %v", got)
	}
	// An SSE server stays one.
	sse := McpEntry{"type": "sse", "url": "https://x.test/sse"}
	if got := EntryOf(FormOf(sse), sse).Type(); got != "sse" {
		t.Errorf("type %q", got)
	}
	// No command yet stays an empty field through the JSON view and back, not `""`.
	if got := FormOf(EntryOf(FormOf(McpEntry{}), McpEntry{})).Command; got != "" {
		t.Errorf("command %q", got)
	}
	if got := FormOf(McpEntry{"command": "", "args": []string{"--flag"}}).Command; got != `"" --flag` {
		t.Errorf("command %q", got)
	}
}

func TestWhatStopsASave(t *testing.T) {
	form := FormOf(McpEntry{"command": "npx"})
	check := func(name string, form McpForm, want string) {
		t.Helper()
		if got := FormProblem(name, form); got != want {
			t.Errorf("FormProblem(%q) = %q, want %q", name, got, want)
		}
	}
	check("", form, "Give the server a name.")
	blank := form
	blank.Command = "   "
	check("fs", blank, "Give the command to run.")
	check("fs", form, "")
	remote := form
	remote.Remote = true
	check("api", remote, "Give the server's URL.")
	remote.URL = "mcp.example.com"
	check("api", remote, "The URL starts with http:// or https://.")
	remote.URL = "${BASE}/mcp"
	check("api", remote, "")
}

func TestEntriesCompareByWhatTheySay(t *testing.T) {
	if !SameEntry(McpEntry{"command": "a", "args": []string{}}, McpEntry{"command": "a"}) {
		t.Error("an empty list is no field")
	}
	if !SameEntry(McpEntry{"env": map[string]string{"B": "2", "A": "1"}, "command": "a"}, McpEntry{"command": "a", "env": map[string]string{"A": "1", "B": "2"}}) {
		t.Error("key order matters")
	}
	if SameEntry(McpEntry{"command": "a", "args": []string{"x"}}, McpEntry{"command": "a"}) {
		t.Error("args ignored")
	}
	want := "{\n  \"command\": \"npx\",\n  \"args\": [\n    \"-y\"\n  ],\n  \"trust\": true\n}"
	if got := EntryJSON(McpEntry{"args": []string{"-y"}, "command": "npx", "trust": true}); got != want {
		t.Errorf("EntryJSON = %s", got)
	}
	if got := EntryJSON(McpEntry{"url": "https://x.test/?a=1&b=2"}); got != "{\n  \"url\": \"https://x.test/?a=1&b=2\"\n}" {
		t.Errorf("EntryJSON escapes: %s", got)
	}
}

func TestSecretsAndServerStates(t *testing.T) {
	for _, name := range []string{"API_KEY", "GITHUB_PERSONAL_ACCESS_TOKEN", "Authorization", "client_secret", "DB_PASSWORD"} {
		if !LooksSecret(name) {
			t.Errorf("%s does not look secret", name)
		}
	}
	for _, name := range []string{"DEBUG", "PORT", "X-Team", "ROOT_DIR"} {
		if LooksSecret(name) {
			t.Errorf("%s looks secret", name)
		}
	}
	if got := McpAddress(McpEntry{"command": "npx", "args": []string{"-y", "My Folder"}}); got != `npx -y "My Folder"` {
		t.Errorf("address %s", got)
	}
	if got := McpAddress(McpEntry{"type": "http", "url": "https://x.test/mcp"}); got != "https://x.test/mcp" {
		t.Errorf("address %s", got)
	}
	server := McpServer{Name: "fs", ID: "fs", Enabled: true, Entry: McpEntry{"command": "npx"}, ToolCount: -1}
	status := func(state PluginState, detail string) *InstalledPlugin {
		return &InstalledPlugin{ID: "fs", Name: "fs", Icon: "terminal", State: state, Detail: detail, Source: "mcp.json"}
	}
	state := func(s McpServer) string { text, _ := McpStateText(s); return text }
	one, many, never, broken, off := server, server, server, server, server
	one.Status, one.ToolCount = status(PluginReady, "Ready"), 1
	many.Status, many.ToolCount = status(PluginReady, "Ready"), 14
	never.Status = status(PluginReady, "Ready")
	broken.Status = status(PluginError, "Cannot start npx")
	off.Enabled = false
	for _, test := range []struct {
		server McpServer
		want   string
	}{{one, "1 tool"}, {many, "14 tools"}, {never, "Not connected yet"}, {broken, "Cannot start npx"}, {off, "Off"}} {
		if got := state(test.server); got != test.want {
			t.Errorf("state %q, want %q", got, test.want)
		}
	}
	problem := server
	problem.Problem = "Give the server a command to run or a URL to connect to."
	if _, tone := McpStateText(problem); tone != ToneRed {
		t.Errorf("a problem's tone is %v", tone)
	}
}

func TestDemoReadsPastedJSON(t *testing.T) {
	servers, err := ParseLocally(`{"command": "npx"}`)
	if err != nil || len(servers) != 1 || !reflect.DeepEqual(servers[0].Entry, McpEntry{"command": "npx"}) {
		t.Errorf("servers %+v, %v", servers, err)
	}
	servers, _ = ParseLocally(`{"mcpServers": {"a": {"url": "https://a.test"}, "b": {"command": "b"}}}`)
	if len(servers) != 2 || servers[0].Name != "a" || servers[1].Name != "b" {
		t.Errorf("servers %+v", servers)
	}
	if _, err := ParseLocally(`{"theme": "dark"}`); err == nil {
		t.Error("no error for JSON without servers")
	}
}
