package model

import (
	"strings"
	"unicode"
	"unicode/utf8"
)

// Bare links in message text, as `crates/markdown/src/autolink.rs` finds them for the macOS and
// phone apps: GitHub's autolinks (`http://` and `https://` URLs, `www.` hosts, email addresses),
// with a `www.` link allowed after any non-ASCII character and a link ending at CJK or general
// punctuation as it ends at a space. The rules and their cases are the crate's.

// Autolink is a bare link in a run of text: the byte range it covers and where it goes.
type Autolink struct {
	Start int
	End   int
	Href  string
}

func isAlphanumeric(r rune) bool {
	return unicode.IsLetter(r) || unicode.IsNumber(r) || unicode.Is(unicode.Other_Alphabetic, r)
}
func isASCIIAlpha(c byte) bool { return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' }
func isASCIIAlnum(c byte) bool { return isASCIIAlpha(c) || c >= '0' && c <= '9' }

// endsLink is a space, `<`, or general or CJK punctuation, which a link stops before.
func endsLink(r rune) bool {
	switch {
	case unicode.Is(unicode.White_Space, r), r == '<':
		return true
	case r >= 0x2000 && r <= 0x206F, r >= 0x3000 && r <= 0x303F:
		return true
	case r >= 0xFF01 && r <= 0xFF0F, r >= 0xFF1A && r <= 0xFF20, r >= 0xFF3B && r <= 0xFF40, r >= 0xFF5B && r <= 0xFF65:
		return true
	}
	return false
}

// FindAutolinks is the bare links in `text`, in order.
func FindAutolinks(text string) []Autolink {
	var links []Autolink
	// The end of the last link: an address never reaches back into it.
	floor := 0
	for i := 0; i < len(text); {
		var found *Autolink
		switch text[i] {
		case 'h', 'H':
			found = urlLink(text, i)
		case 'w':
			found = wwwLink(text, i)
		case '@':
			found = emailLink(text, i, floor)
		}
		if found != nil {
			links = append(links, *found)
			i, floor = found.End, found.End
		} else {
			i++
		}
	}
	return links
}

// urlLink is `http://` or `https://`, in any case, then a host. GitHub reads back over letters to
// find the scheme, so `xhttp://` is not a link.
func urlLink(text string, start int) *Autolink {
	rest := strings.ToLower(text[start:min(len(text), start+8)])
	var from int
	switch {
	case strings.HasPrefix(rest, "https://"):
		from = start + 8
	case strings.HasPrefix(rest, "http://"):
		from = start + 7
	default:
		return nil
	}
	if start > 0 && isASCIIAlpha(text[start-1]) {
		return nil
	}
	first, size := utf8.DecodeRuneInString(text[from:])
	if size == 0 || !isAlphanumeric(first) || !validHost(text, from) {
		return nil
	}
	end := start + trimLink(text[start:extendLink(text, start)])
	return &Autolink{Start: start, End: end, Href: text[start:end]}
}

// wwwLink is `www.` and a host, at the start of the text or after a space, one of `*_~(`, or a
// non-ASCII character. It opens over `http://`.
func wwwLink(text string, start int) *Autolink {
	if !strings.HasPrefix(text[start:], "www.") {
		return nil
	}
	if start > 0 {
		before, _ := utf8.DecodeLastRuneInString(text[:start])
		if !(unicode.Is(unicode.White_Space, before) || strings.ContainsRune("*_~(", before) || before > 0x7f) {
			return nil
		}
	}
	if !validHost(text, start) {
		return nil
	}
	end := start + trimLink(text[start:extendLink(text, start)])
	if end <= start+4 {
		return nil
	}
	return &Autolink{Start: start, End: end, Href: "http://" + text[start:end]}
}

func isNameChar(c byte) bool { return isASCIIAlnum(c) || c == '.' || c == '+' || c == '-' || c == '_' }

// emailLink is an address around the `@` at `at`: letters, digits, and `.+-_` before it, and after
// it labels of letters, digits, `-`, and `_` between dots, at least two, ending in a letter. It
// opens over `mailto:`. One that follows a `/` is part of a path, not an address.
func emailLink(text string, at, floor int) *Autolink {
	start := at
	for start > floor && isNameChar(text[start-1]) {
		start--
	}
	if start == at || (start > 0 && text[start-1] == '/') {
		return nil
	}
	end := at + 1
	dots := 0
	for ; end < len(text); end++ {
		c := text[end]
		if c == '-' || c == '_' || isASCIIAlnum(c) {
			continue
		}
		if c == '.' && end+1 < len(text) && isASCIIAlnum(text[end+1]) {
			dots++
			continue
		}
		if c == '@' {
			return nil
		}
		break
	}
	if dots == 0 || !isASCIIAlpha(text[end-1]) {
		return nil
	}
	return &Autolink{Start: start, End: end, Href: "mailto:" + text[start:end]}
}

// validHost is whether the host name at `from`, letters, digits, hyphens, and underscores between
// dots, has no underscore in its last two labels.
func validHost(text string, from int) bool {
	last, previous := false, false
	for _, c := range text[from:] {
		if c == '.' {
			previous, last = last, false
		} else if c == '_' {
			last = true
		} else if c != '-' && !isAlphanumeric(c) {
			break
		}
	}
	return !last && !previous
}

// extendLink is where a link that starts at `start` runs to: the next space, `<`, or CJK or
// general punctuation.
func extendLink(text string, start int) int {
	for i, r := range text[start:] {
		if endsLink(r) {
			return start + i
		}
	}
	return len(text)
}

// trimLink is the length of `link` without its trailing punctuation: `?!.,:*_~'"`, a `)` with no
// `(` to close, a `;`, or an entity such as `&hl;` whole.
func trimLink(link string) int {
	opening := strings.Count(link, "(")
	closing := strings.Count(link, ")")
	end := len(link)
	for end > 0 {
		c := link[end-1]
		switch {
		case c == ')' && closing > opening:
			closing--
			end--
		case strings.IndexByte("?!.,:*_~'\"", c) >= 0:
			end--
		case c == ';':
			name := 0
			for end-name-2 >= 0 && isASCIIAlpha(link[end-name-2]) {
				name++
			}
			if name > 0 && end >= name+2 && link[end-name-2] == '&' {
				end -= name + 2
			} else {
				end--
			}
		default:
			return end
		}
	}
	return end
}
