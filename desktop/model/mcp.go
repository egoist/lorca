package model

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"regexp"
	"slices"
	"sort"
	"strings"
	"unicode"
)

// The MCP servers a Runner keeps in its mcp.json, after the macOS app's McpServers.swift: each
// one's entry as the file holds it, how it stands, and the tools it offered; the command line the
// server sheet's Command field reads and writes; and the form the sheet edits an entry through.

// McpEntry is one server's entry, canonical: `command`, `args`, `env`, and `cwd` for a command;
// `type`, `url`, and `headers` for a remote server; and any field Lorca does not know, as the
// file has it. Strings are strings, `args` a []string, `env` and `headers` map[string]string.
type McpEntry map[string]any

func (e McpEntry) str(key string) string {
	value, _ := e[key].(string)
	return value
}

func (e McpEntry) has(key string) bool { _, ok := e[key]; return ok }

func (e McpEntry) Type() string        { return e.str("type") }
func (e McpEntry) Command() string     { return e.str("command") }
func (e McpEntry) URL() string         { return e.str("url") }
func (e McpEntry) Description() string { return e.str("description") }
func (e McpEntry) Disabled() bool      { value, _ := e["disabled"].(bool); return value }

func (e McpEntry) Args() []string {
	value, _ := e["args"].([]string)
	return value
}

func (e McpEntry) stringMap(key string) map[string]string {
	value, _ := e[key].(map[string]string)
	return value
}

func (e McpEntry) Env() map[string]string     { return e.stringMap("env") }
func (e McpEntry) Headers() map[string]string { return e.stringMap("headers") }

// Clone copies the entry's top level.
func (e McpEntry) Clone() McpEntry {
	out := make(McpEntry, len(e))
	for key, value := range e {
		out[key] = value
	}
	return out
}

// McpTool is one of a server's tools, as it offered it when it last connected.
type McpTool struct {
	Name  string
	Title string
	// Description is the first line of what it does.
	Description string
	// ReadOnly is the server marking it read-only: it runs without Auto-review.
	ReadOnly bool
	// Hidden is kept from bots by the entry's `toolExposure`.
	Hidden bool
}

// McpServer is a server in a Runner's mcp.json, usable or not.
type McpServer struct {
	// Name is its key in the file: what the apps show and `lorca mcp` takes.
	Name string
	// ID is the plugin id it runs as, which its tools go by (`id__tool`).
	ID      string
	Enabled bool
	Entry   McpEntry
	// Problem is why it cannot run: an entry the CLI cannot read, or a name another plugin's id takes.
	Problem string
	// Status is its plugin's state while it runs and is on.
	Status *InstalledPlugin
	// SignsIn and SignedIn: a remote server that asked for a sign-in, or is signed in.
	SignsIn  bool
	SignedIn bool
	// ToolCount is how many tools it offered when it last connected; -1 before it has.
	ToolCount int
	// Tools are `mcp.get`'s: the tools themselves.
	Tools []McpTool
}

// McpFile is a Runner's mcp.json: where it is, why it cannot be read, and its servers in the
// file's order.
type McpFile struct {
	Path    string
	Error   string
	Servers []McpServer
}

// ParsedServer is one server of pasted JSON: its name when the JSON gives one, and its entry or
// why it cannot run.
type ParsedServer struct {
	Name    string
	Entry   McpEntry
	Problem string
}

func (e McpEntry) IsRemote() bool { return e.has("url") && e.Type() != "stdio" }

func (e McpEntry) Symbol() string {
	if e.IsRemote() {
		return "globe"
	}
	return "terminal"
}

// McpStateText is how a server stands, in a few words, and the tone they take.
func McpStateText(server McpServer) (string, Tone) {
	if server.Problem != "" {
		return server.Problem, ToneRed
	}
	if !server.Enabled {
		return L("Off"), ToneTertiary
	}
	status := server.Status
	if status == nil {
		return "", ToneSecondary
	}
	switch status.State {
	case PluginReady:
		if server.ToolCount < 0 {
			return L("Not connected yet"), ToneSecondary
		}
		if server.ToolCount == 1 {
			return L("1 tool"), ToneGreen
		}
		return L("%d tools", server.ToolCount), ToneGreen
	case PluginNeedsAuth:
		return L("Needs a sign-in"), ToneOrange
	case PluginConnecting:
		if status.Detail != "" {
			return status.Detail, ToneAccent
		}
		return L("Connecting…"), ToneAccent
	case PluginError:
		if status.Detail != "" {
			return status.Detail, ToneRed
		}
		return L("Couldn't connect"), ToneRed
	}
	return status.Detail, ToneSecondary
}

// McpAddress is the command line, or the URL, a row shows under the server's name.
func McpAddress(entry McpEntry) string {
	if entry.IsRemote() {
		return entry.URL()
	}
	return JoinCommandLine(append([]string{entry.Command()}, entry.Args()...))
}

// MARK: - The command line

// SplitCommandLine reads `npx -y "My Folder"` as its words, the way Windows and a shell both read
// it: spaces part words, double quotes group them with `\"` for a quote inside, single quotes group
// them as typed, and a backslash is a backslash, which Windows paths need, except in a run before
// a quote, which halves (Windows' own rule).
func SplitCommandLine(text string) []string {
	var words []string
	var word strings.Builder
	started := false
	var quote rune
	runes := []rune(text)
	for i := 0; i < len(runes); {
		c := runes[i]
		if quote == '\'' {
			if c == '\'' {
				quote = 0
			} else {
				word.WriteRune(c)
			}
			i++
			continue
		}
		if c == '\\' {
			run := 0
			for i+run < len(runes) && runes[i+run] == '\\' {
				run++
			}
			if i+run < len(runes) && runes[i+run] == '"' {
				// 2n backslashes and a quote are n backslashes and the quote; 2n + 1 are n and a quote mark.
				word.WriteString(strings.Repeat("\\", run/2))
				if run%2 == 1 {
					word.WriteByte('"')
					i += run + 1
				} else {
					i += run
				}
			} else {
				word.WriteString(strings.Repeat("\\", run))
				i += run
			}
			started = true
			continue
		}
		switch {
		case c == '"':
			if quote == '"' {
				quote = 0
			} else {
				quote = '"'
			}
			started = true
		case quote == 0 && c == '\'':
			quote = '\''
			started = true
		case quote == 0 && unicode.IsSpace(c):
			if started {
				words = append(words, word.String())
			}
			word.Reset()
			started = false
		default:
			word.WriteRune(c)
			started = true
		}
		i++
	}
	if started {
		words = append(words, word.String())
	}
	return words
}

// JoinCommandLine is words as one command line that SplitCommandLine reads back as the same
// words: a word with a space, a quote, or nothing in it in double quotes.
func JoinCommandLine(words []string) string {
	quoted := make([]string, 0, len(words))
	for _, word := range words {
		quoted = append(quoted, quoteWord(word))
	}
	return strings.Join(quoted, " ")
}

func quoteWord(word string) string {
	if word != "" && !strings.ContainsFunc(word, func(r rune) bool { return unicode.IsSpace(r) || r == '"' || r == '\'' }) {
		return word
	}
	var out strings.Builder
	out.WriteByte('"')
	backslashes := 0
	for _, c := range word {
		if c == '\\' {
			backslashes++
			continue
		}
		if c == '"' {
			out.WriteString(strings.Repeat("\\", backslashes*2+1))
		} else {
			out.WriteString(strings.Repeat("\\", backslashes))
		}
		out.WriteRune(c)
		backslashes = 0
	}
	out.WriteString(strings.Repeat("\\", backslashes*2))
	out.WriteByte('"')
	return out.String()
}

// MARK: - The sheet's form

var secretName = regexp.MustCompile(`(?i)key|token|secret|passw|auth|credential|cookie|session`)

// LooksSecret is a name that says its value is a key, a token, or a password, which the sheet hides.
func LooksSecret(name string) bool { return secretName.MatchString(name) }

// Pair is one row of the environment or the headers.
type Pair struct {
	Name  string
	Value string
}

// McpForm is an entry as the server sheet's form holds it: the command line as typed, and the
// environment or headers as rows.
type McpForm struct {
	Remote      bool
	Command     string
	Env         []Pair
	URL         string
	Headers     []Pair
	Description string
}

func pairs(values map[string]string) []Pair {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	out := make([]Pair, 0, len(keys))
	for _, key := range keys {
		out = append(out, Pair{key, values[key]})
	}
	return out
}

func FormOf(entry McpEntry) McpForm {
	// No command yet is an empty field, not the quoted empty word `""`.
	words := append([]string{entry.Command()}, entry.Args()...)
	command := JoinCommandLine(words)
	if len(words) == 1 && words[0] == "" {
		command = ""
	}
	return McpForm{
		Remote:      entry.IsRemote(),
		Command:     command,
		Env:         pairs(entry.Env()),
		URL:         entry.URL(),
		Headers:     pairs(entry.Headers()),
		Description: entry.Description(),
	}
}

// formKeys are the keys the form owns; the entry's others (`cwd`, `oauth`, `disabled`, and fields
// other apps write) stay as they are.
var formKeys = []string{"type", "command", "args", "env", "environment", "url", "serverUrl", "httpUrl", "headers", "description"}

func rows(list []Pair) map[string]string {
	out := map[string]string{}
	for _, pair := range list {
		if name := strings.TrimSpace(pair.Name); name != "" {
			out[name] = pair.Value
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

// EntryOf is the entry the form describes, over `base`'s other fields.
func EntryOf(form McpForm, base McpEntry) McpEntry {
	entry := McpEntry{}
	if form.Remote {
		entry["type"] = "http"
		if base.Type() == "sse" {
			entry["type"] = "sse"
		}
		entry["url"] = strings.TrimSpace(form.URL)
		if headers := rows(form.Headers); headers != nil {
			entry["headers"] = headers
		}
	} else {
		if base.Type() == "stdio" {
			entry["type"] = "stdio"
		}
		words := SplitCommandLine(strings.TrimSpace(form.Command))
		command := ""
		if len(words) > 0 {
			command, words = words[0], words[1:]
		}
		entry["command"] = command
		if len(words) > 0 {
			entry["args"] = words
		}
		if env := rows(form.Env); env != nil {
			entry["env"] = env
		}
	}
	if description := strings.TrimSpace(form.Description); description != "" {
		entry["description"] = description
	}
	for key, value := range base {
		if !slices.Contains(formKeys, key) && value != nil {
			entry[key] = value
		}
	}
	return entry
}

// FormProblem is why the form cannot be saved yet, or "".
func FormProblem(name string, form McpForm) string {
	if strings.TrimSpace(name) == "" {
		return L("Give the server a name.")
	}
	if form.Remote {
		url := strings.TrimSpace(form.URL)
		if url == "" {
			return L("Give the server's URL.")
		}
		if !(strings.HasPrefix(url, "http://") || strings.HasPrefix(url, "https://") || strings.HasPrefix(url, "${")) {
			return L("The URL starts with http:// or https://.")
		}
		return ""
	}
	if len(SplitCommandLine(strings.TrimSpace(form.Command))) == 0 {
		return L("Give the command to run.")
	}
	return ""
}

// normal is a value with its empty lists and objects left out, in plain JSON shapes.
func normal(value any) any {
	switch v := value.(type) {
	case McpEntry:
		return normal(map[string]any(v))
	case map[string]any:
		out := map[string]any{}
		for key, field := range v {
			if !isEmptyField(field) {
				out[key] = normal(field)
			}
		}
		return out
	case map[string]string:
		out := map[string]any{}
		for key, field := range v {
			out[key] = field
		}
		return out
	case []string:
		out := make([]any, len(v))
		for i, each := range v {
			out[i] = each
		}
		return out
	case []any:
		out := make([]any, len(v))
		for i, each := range v {
			out[i] = normal(each)
		}
		return out
	}
	return value
}

func isEmptyField(value any) bool {
	if value == nil {
		return true
	}
	rv := reflect.ValueOf(value)
	switch rv.Kind() {
	case reflect.Slice, reflect.Map:
		return rv.Len() == 0
	}
	return false
}

// SameEntry is whether two entries say the same, whatever the order of their keys; an empty list
// or object is no field at all.
func SameEntry(a, b McpEntry) bool {
	left, _ := json.Marshal(normal(a))
	right, _ := json.Marshal(normal(b))
	return bytes.Equal(left, right)
}

// EntryJSON is an entry as the JSON field shows it: the keys in the order Lorca writes them, then
// the rest.
func EntryJSON(entry McpEntry) string {
	order := []string{"type", "command", "args", "env", "cwd", "url", "headers", "oauth", "description", "disabled"}
	var keys []string
	for _, key := range order {
		if entry[key] != nil {
			keys = append(keys, key)
		}
	}
	var rest []string
	for key, value := range entry {
		if value != nil && !slices.Contains(order, key) {
			rest = append(rest, key)
		}
	}
	sort.Strings(rest)
	keys = append(keys, rest...)
	var b bytes.Buffer
	b.WriteString("{")
	for i, key := range keys {
		if i > 0 {
			b.WriteString(",")
		}
		fmt.Fprintf(&b, "\n  %s: %s", jsonText(key, ""), jsonText(normalForJSON(entry[key]), "  "))
	}
	if len(keys) > 0 {
		b.WriteString("\n")
	}
	b.WriteString("}")
	return b.String()
}

// jsonText is a value as JSON, indented by two under `prefix`, with its URLs' `&` and `<` as typed.
func jsonText(value any, prefix string) string {
	var b bytes.Buffer
	encoder := json.NewEncoder(&b)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent(prefix, "  ")
	_ = encoder.Encode(value)
	return strings.TrimSuffix(b.String(), "\n")
}

func normalForJSON(value any) any {
	if v, ok := value.(McpEntry); ok {
		return map[string]any(v)
	}
	return value
}

// ToMcpEntry is an entry as the apps hold it: the strings where strings belong, whatever else the
// CLI sent.
func ToMcpEntry(wire map[string]any) McpEntry {
	entry := McpEntry{}
	for key, value := range wire {
		entry[key] = value
	}
	text := func(value any) (string, bool) {
		switch v := value.(type) {
		case nil:
			return "", false
		case string:
			return v, true
		case float64:
			return fmt.Sprint(v), true
		case bool:
			return fmt.Sprint(v), true
		}
		data, _ := json.Marshal(value)
		return string(data), true
	}
	for _, key := range []string{"type", "command", "cwd", "url", "description"} {
		if value, ok := text(entry[key]); ok {
			entry[key] = value
		} else {
			delete(entry, key)
		}
	}
	switch args := entry["args"].(type) {
	case []any:
		words := make([]string, 0, len(args))
		for _, arg := range args {
			word, _ := text(arg)
			words = append(words, word)
		}
		entry["args"] = words
	case []string:
	default:
		delete(entry, "args")
	}
	for _, key := range []string{"env", "headers"} {
		switch values := entry[key].(type) {
		case map[string]any:
			out := make(map[string]string, len(values))
			for name, value := range values {
				out[name], _ = text(value)
			}
			entry[key] = out
		case map[string]string:
		default:
			delete(entry, key)
		}
	}
	if disabled, _ := entry["disabled"].(bool); disabled {
		entry["disabled"] = true
	} else {
		delete(entry, "disabled")
	}
	for key, value := range entry {
		if value == nil {
			delete(entry, key)
		}
	}
	return entry
}

// ParseLocally is what the demo, which has no CLI, makes of pasted JSON: a server, or servers by
// name under `mcpServers` or `servers`. The CLI reads every other app's shape (`mcp.parse`).
func ParseLocally(text string) ([]ParsedServer, error) {
	var value any
	if err := json.Unmarshal([]byte(text), &value); err != nil {
		return nil, err
	}
	object, ok := value.(map[string]any)
	if !ok {
		return nil, errors.New(L("Not a server's JSON: expected an object."))
	}
	looksLikeServer := func(candidate any) bool {
		server, ok := candidate.(map[string]any)
		if !ok {
			return false
		}
		_, command := server["command"]
		_, url := server["url"]
		return command || url
	}
	if looksLikeServer(object) {
		return []ParsedServer{{Entry: ToMcpEntry(object)}}, nil
	}
	container := object
	if servers, ok := object["mcpServers"].(map[string]any); ok {
		container = servers
	} else if servers, ok := object["servers"].(map[string]any); ok {
		container = servers
	}
	var names []string
	for name, entry := range container {
		if looksLikeServer(entry) {
			names = append(names, name)
		}
	}
	if len(names) == 0 {
		return nil, errors.New(L("No MCP servers in that JSON: expected mcpServers, a server's command, or its url."))
	}
	sort.Strings(names)
	out := make([]ParsedServer, 0, len(names))
	for _, name := range names {
		out = append(out, ParsedServer{Name: name, Entry: ToMcpEntry(container[name].(map[string]any))})
	}
	return out, nil
}
