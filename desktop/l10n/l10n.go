// Package l10n holds the app's words in the user's language, after the macOS app's `L()`. The key
// is the English text, so a string with no translation reads as written; the table is zh.go, which
// `bun run l10n` checks against every `L("…")` and `Lc("…", "…")` in the sources.
//
// The language can change while the app runs: `L()` reads the table in force, and the views, which
// are built for every frame, read it again. So `L()` is called when a view is built and its words
// never kept in a package-level variable.
package l10n

import (
	"strconv"
	"strings"
	"sync/atomic"
)

// Language is a language the app speaks: "en" or "zh-Hans".
type Language string

const (
	English Language = "en"
	Chinese Language = "zh-Hans"
)

// Supported are the languages Settings offers, by their own names.
var Supported = []struct {
	Code Language
	Name string
}{
	{English, "English"},
	{Chinese, "简体中文"},
}

type state struct {
	chosen Language // "" follows the system
	system string   // the system's locale, such as "zh-CN"
}

var current atomic.Pointer[state]

func init() { current.Store(&state{system: "en-US"}) }

// Set applies the system's locale and the pick ("en", "zh-Hans", or "" to follow the system), and
// reports whether the words changed.
func Set(pick, system string) bool {
	before := Code()
	next := &state{system: current.Load().system}
	if system != "" {
		next.system = system
	}
	switch Language(pick) {
	case English, Chinese:
		next.chosen = Language(pick)
	}
	current.Store(next)
	return before != Code()
}

// Code is the language in force: the one picked in Settings, else the system's.
func Code() Language {
	s := current.Load()
	if s.chosen != "" {
		return s.chosen
	}
	if strings.HasPrefix(strings.ToLower(s.system), "zh") {
		return Chinese
	}
	return English
}

// Chosen is the pick, or "" when the app follows the system.
func Chosen() Language { return current.Load().chosen }

// IsChinese is whether the app speaks Simplified Chinese.
func IsChinese() bool { return Code() == Chinese }

func lookup(key string) string {
	if IsChinese() {
		if text, ok := zh[key]; ok {
			return text
		}
	}
	return key
}

// L is `L("New Bot…")`, or with values: `L("%@ is working…", name)`.
func L(key string, args ...any) string {
	text := lookup(key)
	if len(args) == 0 {
		return text
	}
	return Format(text, args...)
}

// Lc is one English word that means two things: `Lc("Pairing", "device state")` looks up
// `Pairing|device state`, and reads as "Pairing" where that has no translation.
func Lc(text, context string) string {
	if IsChinese() {
		if found, ok := zh[text+"|"+context]; ok {
			return found
		}
	}
	return text
}

// Format fills `%@` (text) and `%d` (whole numbers) in order, or `%1$@`, `%2$d` by position, and
// reads `%%` as a percent sign.
func Format(template string, args ...any) string {
	var out strings.Builder
	next := 0
	for i := 0; i < len(template); i++ {
		c := template[i]
		if c != '%' || i+1 >= len(template) {
			out.WriteByte(c)
			continue
		}
		j := i + 1
		position := -1
		if k := j; k < len(template) && template[k] >= '1' && template[k] <= '9' {
			for k < len(template) && template[k] >= '0' && template[k] <= '9' {
				k++
			}
			if k < len(template) && template[k] == '$' {
				position, _ = strconv.Atoi(template[j:k])
				j = k + 1
			}
		}
		if j >= len(template) {
			out.WriteString(template[i:])
			break
		}
		kind := template[j]
		switch kind {
		case '%':
			out.WriteByte('%')
			i = j
			continue
		case '@', 'd', 's':
		default:
			out.WriteByte(c)
			continue
		}
		index := next
		if position > 0 {
			index = position - 1
		} else {
			next++
		}
		if index >= len(args) {
			out.WriteString(template[i : j+1])
		} else if kind == 'd' {
			out.WriteString(integer(args[index]))
		} else {
			out.WriteString(text(args[index]))
		}
		i = j
	}
	return out.String()
}

func integer(value any) string {
	switch v := value.(type) {
	case int:
		return strconv.Itoa(v)
	case int64:
		return strconv.FormatInt(v, 10)
	case int32:
		return strconv.FormatInt(int64(v), 10)
	case uint:
		return strconv.FormatUint(uint64(v), 10)
	case uint64:
		return strconv.FormatUint(v, 10)
	case float64:
		return strconv.FormatInt(int64(v), 10)
	case float32:
		return strconv.FormatInt(int64(v), 10)
	}
	return text(value)
}

func text(value any) string {
	switch v := value.(type) {
	case string:
		return v
	case int, int64, int32, uint, uint64, float64, float32:
		return integer(v)
	case interface{ String() string }:
		return v.String()
	}
	return ""
}
