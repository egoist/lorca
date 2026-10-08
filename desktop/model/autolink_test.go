package model

import (
	"reflect"
	"testing"
)

// The same cases as crates/markdown's autolink tests, so this app links what the others do.

// links is each link as [its text, its href].
func links(text string) [][2]string {
	var out [][2]string
	for _, link := range FindAutolinks(text) {
		out = append(out, [2]string{text[link.Start:link.End], link.Href})
	}
	return out
}

func one(t *testing.T, text string) [2]string {
	t.Helper()
	found := links(text)
	if len(found) != 1 {
		t.Fatalf("%q: %d links %v", text, len(found), found)
	}
	return found[0]
}

func none(t *testing.T, texts ...string) {
	t.Helper()
	for _, text := range texts {
		if found := links(text); len(found) != 0 {
			t.Errorf("%q links %v", text, found)
		}
	}
}

func linksTo(t *testing.T, text, want string) {
	t.Helper()
	if got := one(t, text)[0]; got != want {
		t.Errorf("%q links %q, want %q", text, got, want)
	}
}

func TestWWWLinksOpenOverHTTP(t *testing.T) {
	if got := one(t, "www.commonmark.org"); got != [2]string{"www.commonmark.org", "http://www.commonmark.org"} {
		t.Errorf("got %v", got)
	}
	linksTo(t, "Visit www.commonmark.org/help for more information.", "www.commonmark.org/help")
	linksTo(t, "Visit www.commonmark.org.", "www.commonmark.org")
	linksTo(t, "Visit www.commonmark.org/a.b.", "www.commonmark.org/a.b")
	linksTo(t, "(www.example.com)", "www.example.com")
	none(t, "wwww.example.com", "xwww.example.com", "www.", "WWW.example.com")
}

func TestParenthesesCloseInsideOnlyWhenOpened(t *testing.T) {
	linksTo(t, "www.google.com/search?q=Markup+(business)", "www.google.com/search?q=Markup+(business)")
	linksTo(t, "www.google.com/search?q=Markup+(business)))", "www.google.com/search?q=Markup+(business)")
	linksTo(t, "(www.google.com/search?q=Markup+(business))", "www.google.com/search?q=Markup+(business)")
	linksTo(t, "(www.google.com/search?q=Markup+(business)", "www.google.com/search?q=Markup+(business)")
	linksTo(t, "www.google.com/search?q=(business))+ok", "www.google.com/search?q=(business))+ok")
}

func TestEntitiesAndAngleBracketsEndALink(t *testing.T) {
	linksTo(t, "www.google.com/search?q=commonmark&hl=en", "www.google.com/search?q=commonmark&hl=en")
	linksTo(t, "www.google.com/search?q=commonmark&hl;", "www.google.com/search?q=commonmark")
	linksTo(t, "www.commonmark.org/he<lp", "www.commonmark.org/he")
}

func TestURLsKeepTheirScheme(t *testing.T) {
	if got := one(t, "http://commonmark.org"); got != [2]string{"http://commonmark.org", "http://commonmark.org"} {
		t.Errorf("got %v", got)
	}
	linksTo(t, "(Visit https://encrypted.google.com/search?q=Markup+(business))", "https://encrypted.google.com/search?q=Markup+(business)")
	linksTo(t, "Run it at http://localhost:3000/app.", "http://localhost:3000/app")
	linksTo(t, "HTTPS://Example.com/", "HTTPS://Example.com/")
	linksTo(t, "'https://example.com/a'", "https://example.com/a")
	none(t, "xhttps://example.com", "https://", "https://-x.com", "ftp://example.com")
}

func TestNoUnderscoreInTheLastTwoLabels(t *testing.T) {
	linksTo(t, "www._xxx.yyy.zzz", "www._xxx.yyy.zzz")
	none(t, "www.xxx.yyy._zzz", "www.xxx._yyy.zzz")
	linksTo(t, "https://a.b/c_d_", "https://a.b/c_d")
}

func TestEmailsOpenOverMailto(t *testing.T) {
	if got := one(t, "foo@bar.baz"); got != [2]string{"foo@bar.baz", "mailto:foo@bar.baz"} {
		t.Errorf("got %v", got)
	}
	linksTo(t, "hello@mail+xyz.example isn't valid, but hello+xyz@mail.example is.", "hello+xyz@mail.example")
	linksTo(t, "a.b-c_d@a.b", "a.b-c_d@a.b")
	linksTo(t, "a.b-c_d@a.b.", "a.b-c_d@a.b")
	none(t, "a.b-c_d@a.b-", "a.b-c_d@a.b_", "@bar.baz", "foo@bar", "path/to@bar.baz")
	linksTo(t, "a@b@c.com", "b@c.com")
}

func TestSeveralLinksInARun(t *testing.T) {
	want := [][2]string{{"https://a.com", "https://a.com"}, {"www.b.org", "http://www.b.org"}, {"c@d.io", "mailto:c@d.io"}}
	if got := links("see https://a.com, www.b.org and c@d.io."); !reflect.DeepEqual(got, want) {
		t.Errorf("got %v", got)
	}
}

func TestCJKTextRunsIntoLinks(t *testing.T) {
	linksTo(t, "请访问https://example.com/docs，里面有说明。", "https://example.com/docs")
	linksTo(t, "见www.example.com。", "www.example.com")
	linksTo(t, "（https://zh.wikipedia.org/wiki/北京）", "https://zh.wikipedia.org/wiki/北京")
	linksTo(t, "“https://example.com”", "https://example.com")
	linksTo(t, "联系me@example.com。", "me@example.com")
}
