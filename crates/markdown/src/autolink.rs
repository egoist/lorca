//! Bare links in message text, read as GitHub reads them (the GFM autolink extension, as
//! cmark-gfm implements it): `http://` and `https://` URLs, `www.` hosts, and email addresses.
//! Two rules are Lorca's, for Chinese and Japanese text, which runs into a link without a space:
//! a `www.` link may follow any non-ASCII character, and a link ends at CJK or general
//! punctuation (`，`, `。`, `）`, `“`, `…`) as it ends at a space.
//!
//! `desktop/src/model/autolink.ts` holds the same rules, and these cases, for the Windows and Linux
//! app.

/// A bare link in a run of text: the byte range it covers and where it goes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Autolink {
    pub start: usize,
    pub end: usize,
    pub href: String,
}

/// The bare links in `text`, in order.
pub(crate) fn find(text: &str) -> Vec<Autolink> {
    let bytes = text.as_bytes();
    let mut links = Vec::new();
    let mut i = 0;
    // The end of the last link: an address never reaches back into it.
    let mut floor = 0;
    while i < bytes.len() {
        let found = match bytes[i] {
            b'h' | b'H' => url(text, i),
            b'w' => www(text, i),
            b'@' => email(text, i, floor),
            _ => None,
        };
        match found {
            Some(link) => {
                i = link.end;
                floor = link.end;
                links.push(link);
            }
            None => i += 1,
        }
    }
    links
}

/// `http://` or `https://`, in any case, then a host. GitHub reads back over letters to find
/// the scheme, so `xhttp://` is not a link.
fn url(text: &str, start: usize) -> Option<Autolink> {
    let head = &text.as_bytes()[start..];
    let scheme = [&b"https://"[..], b"http://"].into_iter().find(|scheme| head.get(..scheme.len()).is_some_and(|bytes| bytes.eq_ignore_ascii_case(scheme)))?;
    if text[..start].chars().next_back().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let rest = &text[start + scheme.len()..];
    if !rest.starts_with(char::is_alphanumeric) || !host(rest) {
        return None;
    }
    let end = start + trim(&text[start..extend(text, start)]);
    Some(Autolink { start, end, href: text[start..end].to_owned() })
}

/// `www.` and a host, at the start of the text or after a space, one of `*_~(`, or a non-ASCII
/// character. It opens over `http://`.
fn www(text: &str, start: usize) -> Option<Autolink> {
    if !text[start..].starts_with("www.") {
        return None;
    }
    if let Some(c) = text[..start].chars().next_back() {
        if !(c.is_whitespace() || matches!(c, '*' | '_' | '~' | '(') || !c.is_ascii()) {
            return None;
        }
    }
    if !host(&text[start..]) {
        return None;
    }
    let end = start + trim(&text[start..extend(text, start)]);
    if end <= start + 4 {
        return None;
    }
    Some(Autolink { start, end, href: format!("http://{}", &text[start..end]) })
}

/// An address around the `@` at `at`: letters, digits, and `.+-_` before it, and after it
/// labels of letters, digits, `-`, and `_` between dots, at least two, ending in a letter. It
/// opens over `mailto:`. One that follows a `/` is part of a path, not an address.
fn email(text: &str, at: usize, floor: usize) -> Option<Autolink> {
    let bytes = text.as_bytes();
    let mut start = at;
    while start > floor && (bytes[start - 1].is_ascii_alphanumeric() || matches!(bytes[start - 1], b'.' | b'+' | b'-' | b'_')) {
        start -= 1;
    }
    if start == at || (start > 0 && bytes[start - 1] == b'/') {
        return None;
    }
    let mut end = at + 1;
    let mut dots = 0;
    while end < bytes.len() {
        match bytes[end] {
            b'-' | b'_' => {}
            b if b.is_ascii_alphanumeric() => {}
            b'.' if bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric) => dots += 1,
            b'@' => return None,
            _ => break,
        }
        end += 1;
    }
    if dots == 0 || !bytes[end - 1].is_ascii_alphabetic() {
        return None;
    }
    Some(Autolink { start, end, href: format!("mailto:{}", &text[start..end]) })
}

/// Whether the host name `text` opens with, letters, digits, hyphens, and underscores between
/// dots, has no underscore in its last two labels.
fn host(text: &str) -> bool {
    // Whether the label being read, and the one before it, has an underscore.
    let (mut last, mut previous) = (false, false);
    for c in text.chars() {
        match c {
            '.' => (previous, last) = (last, false),
            '_' => last = true,
            '-' => {}
            c if c.is_alphanumeric() => {}
            _ => break,
        }
    }
    !last && !previous
}

/// Where a link that starts at `start` runs to: the next space, `<`, or CJK or general
/// punctuation.
fn extend(text: &str, start: usize) -> usize {
    text[start..].find(ends_link).map_or(text.len(), |length| start + length)
}

fn ends_link(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '\u{2000}'..='\u{206F}' | '\u{3000}'..='\u{303F}' | '\u{FF01}'..='\u{FF0F}' | '\u{FF1A}'..='\u{FF20}' | '\u{FF3B}'..='\u{FF40}' | '\u{FF5B}'..='\u{FF65}')
}

/// The length of `link` without its trailing punctuation: `?!.,:*_~'"`, a `)` with no `(` to
/// close, a `;`, or an entity such as `&hl;` whole.
fn trim(link: &str) -> usize {
    let bytes = link.as_bytes();
    let opening = bytes.iter().filter(|&&b| b == b'(').count();
    let mut closing = bytes.iter().filter(|&&b| b == b')').count();
    let mut end = bytes.len();
    while end > 0 {
        match bytes[end - 1] {
            b')' if closing > opening => {
                closing -= 1;
                end -= 1;
            }
            b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~' | b'\'' | b'"' => end -= 1,
            b';' => {
                let name = bytes[..end - 1].iter().rev().take_while(|b| b.is_ascii_alphabetic()).count();
                if name > 0 && end >= name + 2 && bytes[end - name - 2] == b'&' {
                    end -= name + 2;
                } else {
                    end -= 1;
                }
            }
            _ => break,
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each link as (its text, its href).
    fn links(text: &str) -> Vec<(&str, String)> {
        find(text).into_iter().map(|link| (&text[link.start..link.end], link.href)).collect()
    }

    fn one(text: &str) -> (&str, String) {
        let mut found = links(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        found.remove(0)
    }

    #[test]
    fn www_links_open_over_http() {
        assert_eq!(one("www.commonmark.org"), ("www.commonmark.org", "http://www.commonmark.org".to_owned()));
        assert_eq!(one("Visit www.commonmark.org/help for more information.").0, "www.commonmark.org/help");
        assert_eq!(one("Visit www.commonmark.org.").0, "www.commonmark.org");
        assert_eq!(one("Visit www.commonmark.org/a.b.").0, "www.commonmark.org/a.b");
        assert_eq!(one("(www.example.com)").0, "www.example.com");
        assert!(links("wwww.example.com").is_empty());
        assert!(links("xwww.example.com").is_empty());
        assert!(links("www.").is_empty());
        assert!(links("WWW.example.com").is_empty());
    }

    #[test]
    fn parentheses_close_inside_the_link_only_when_opened_there() {
        assert_eq!(one("www.google.com/search?q=Markup+(business)").0, "www.google.com/search?q=Markup+(business)");
        assert_eq!(one("www.google.com/search?q=Markup+(business)))").0, "www.google.com/search?q=Markup+(business)");
        assert_eq!(one("(www.google.com/search?q=Markup+(business))").0, "www.google.com/search?q=Markup+(business)");
        assert_eq!(one("(www.google.com/search?q=Markup+(business)").0, "www.google.com/search?q=Markup+(business)");
        assert_eq!(one("www.google.com/search?q=(business))+ok").0, "www.google.com/search?q=(business))+ok");
    }

    #[test]
    fn entities_and_angle_brackets_end_a_link() {
        assert_eq!(one("www.google.com/search?q=commonmark&hl=en").0, "www.google.com/search?q=commonmark&hl=en");
        assert_eq!(one("www.google.com/search?q=commonmark&hl;").0, "www.google.com/search?q=commonmark");
        assert_eq!(one("www.commonmark.org/he<lp").0, "www.commonmark.org/he");
    }

    #[test]
    fn urls_keep_their_scheme() {
        assert_eq!(one("http://commonmark.org"), ("http://commonmark.org", "http://commonmark.org".to_owned()));
        assert_eq!(one("(Visit https://encrypted.google.com/search?q=Markup+(business))").0, "https://encrypted.google.com/search?q=Markup+(business)");
        assert_eq!(one("Run it at http://localhost:3000/app.").0, "http://localhost:3000/app");
        assert_eq!(one("HTTPS://Example.com/").0, "HTTPS://Example.com/");
        assert_eq!(one("'https://example.com/a'").0, "https://example.com/a");
        assert!(links("xhttps://example.com").is_empty());
        assert!(links("https://").is_empty());
        assert!(links("https://-x.com").is_empty());
        assert!(links("ftp://example.com").is_empty());
    }

    #[test]
    fn no_underscore_in_the_last_two_labels() {
        assert_eq!(one("www._xxx.yyy.zzz").0, "www._xxx.yyy.zzz");
        assert!(links("www.xxx.yyy._zzz").is_empty());
        assert!(links("www.xxx._yyy.zzz").is_empty());
        assert_eq!(one("https://a.b/c_d_").0, "https://a.b/c_d");
    }

    #[test]
    fn emails_open_over_mailto() {
        assert_eq!(one("foo@bar.baz"), ("foo@bar.baz", "mailto:foo@bar.baz".to_owned()));
        assert_eq!(one("hello@mail+xyz.example isn't valid, but hello+xyz@mail.example is.").0, "hello+xyz@mail.example");
        assert_eq!(one("a.b-c_d@a.b").0, "a.b-c_d@a.b");
        assert_eq!(one("a.b-c_d@a.b.").0, "a.b-c_d@a.b");
        assert!(links("a.b-c_d@a.b-").is_empty());
        assert!(links("a.b-c_d@a.b_").is_empty());
        assert!(links("@bar.baz").is_empty());
        assert!(links("foo@bar").is_empty());
        assert!(links("path/to@bar.baz").is_empty());
        assert_eq!(one("a@b@c.com").0, "b@c.com");
    }

    #[test]
    fn several_links_in_a_run() {
        let found = links("see https://a.com, www.b.org and c@d.io.");
        assert_eq!(found, vec![("https://a.com", "https://a.com".to_owned()), ("www.b.org", "http://www.b.org".to_owned()), ("c@d.io", "mailto:c@d.io".to_owned())]);
    }

    #[test]
    fn cjk_text_runs_into_links() {
        assert_eq!(one("请访问https://example.com/docs，里面有说明。").0, "https://example.com/docs");
        assert_eq!(one("见www.example.com。").0, "www.example.com");
        assert_eq!(one("（https://zh.wikipedia.org/wiki/北京）").0, "https://zh.wikipedia.org/wiki/北京");
        assert_eq!(one("“https://example.com”").0, "https://example.com");
        assert_eq!(one("联系me@example.com。").0, "me@example.com");
    }
}
