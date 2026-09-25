//! What a terminal would show of a command's raw output, as text for the model and the
//! transcript, after pi's `stripAnsi` and `sanitizeBinaryOutput`: escape sequences (colors,
//! cursor movement, titles, hyperlinks) are dropped, a carriage return starts its line over the
//! way a progress bar redraws itself, a backspace takes back the character before it, and
//! control characters and other binary junk go. Line breaks and tabs stay, so line counts match
//! the raw output.

/// The text of `raw`, a slice of terminal output. A slice that starts inside a UTF-8 character
/// or an escape sequence (the head of a buffer that dropped older bytes) loses that fragment.
pub fn terminal_text(raw: &[u8]) -> String {
    let decoded = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(decoded.len());
    // Where the current line starts in `out`, for carriage returns and backspaces.
    let mut line_start = 0;
    // A carriage return waits for the next character: before a line feed it is part of CRLF;
    // before anything else the line is written over.
    let mut pending_return = false;
    let mut chars = decoded.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => skip_escape(&mut chars),
            // C1 controls as characters: CSI, OSC, and the string introducers.
            '\u{9b}' => skip_csi(&mut chars),
            '\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => skip_string(&mut chars),
            '\r' => pending_return = true,
            '\n' => {
                pending_return = false;
                out.push('\n');
                line_start = out.len();
            }
            '\u{8}' => {
                if out.len() > line_start {
                    out.pop();
                }
            }
            c if is_junk(c) => {}
            c => {
                if pending_return {
                    pending_return = false;
                    out.truncate(line_start);
                }
                out.push(c);
            }
        }
    }
    out
}

/// Control characters other than tab and line feed, the C1 range, and the Unicode
/// interlinear annotation characters pi also drops.
fn is_junk(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{fff9}'..='\u{fffb}')
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// After ESC: a CSI (`ESC [`), a string (OSC `ESC ]`, DCS `ESC P`, SOS `ESC X`, PM `ESC ^`, APC
/// `ESC _`), or a short sequence of intermediates and one final character (`ESC ( B`, `ESC 7`).
fn skip_escape(chars: &mut Chars) {
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            skip_csi(chars);
        }
        Some(']' | 'P' | 'X' | '^' | '_') => {
            chars.next();
            skip_string(chars);
        }
        Some(c) if ('\u{20}'..='\u{2f}').contains(&c) => {
            while chars.next_if(|c| ('\u{20}'..='\u{2f}').contains(c)).is_some() {}
            chars.next_if(|c| ('\u{30}'..='\u{7e}').contains(c));
        }
        Some(c) if ('\u{30}'..='\u{7e}').contains(&c) => {
            chars.next();
        }
        _ => {}
    }
}

/// Parameters and intermediates up to the final character, `ESC[1;31m`, `ESC[?25l`, `ESC[2K`.
/// Anything else ends a sequence that was cut off, and stays.
fn skip_csi(chars: &mut Chars) {
    while chars.next_if(|c| ('\u{20}'..='\u{3f}').contains(c)).is_some() {}
    chars.next_if(|c| ('\u{40}'..='\u{7e}').contains(c));
}

/// Up to the string terminator: BEL, `ESC \`, or ST. OSC 8 hyperlinks and window titles. A
/// line break ends one that never terminates, so a stray introducer cannot hide what follows.
fn skip_string(chars: &mut Chars) {
    while let Some(c) = chars.next_if(|c| *c != '\n') {
        match c {
            '\u{7}' | '\u{9c}' => return,
            '\u{1b}' => {
                chars.next_if_eq(&'\\');
                return;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escape_sequences() {
        assert_eq!(terminal_text(b"\x1b[1;31merror\x1b[0m: nope"), "error: nope");
        assert_eq!(terminal_text(b"\x1b[?25l\x1b[2Kdone\x1b[?25h"), "done");
        assert_eq!(terminal_text(b"\x1b]0;title\x07after"), "after");
        assert_eq!(terminal_text(b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\"), "link");
        assert_eq!(terminal_text(b"\x1b(Bplain\x1b7\x1b8"), "plain");
        assert_eq!(terminal_text("\u{9b}32mgreen".as_bytes()), "green");
        assert_eq!(terminal_text(b"cut\x1b[12\nnext"), "cut\nnext");
        assert_eq!(terminal_text(b"\x1b]unterminated\nstill here"), "\nstill here");
    }

    #[test]
    fn carriage_returns_redraw_the_line_and_crlf_is_a_line_break() {
        assert_eq!(terminal_text(b"one\r\ntwo\r\n"), "one\ntwo\n");
        assert_eq!(terminal_text(b"10%\r50%\r100%\ndone"), "100%\ndone");
        assert_eq!(terminal_text(b"\r\x1b[Kbuilding\r\x1b[K   Compiling a\r\n"), "   Compiling a\n");
        assert_eq!(terminal_text(b"Password: \r\n"), "Password: \n");
        assert_eq!(terminal_text(b"waiting\r"), "waiting", "a trailing return has nothing to write over yet");
    }

    #[test]
    fn backspaces_and_control_characters() {
        assert_eq!(terminal_text(b"_\x08u_\x08n"), "un");
        assert_eq!(terminal_text(b"|\x08/\x08-\x08ok"), "ok");
        assert_eq!(terminal_text(b"\n\x08x"), "\nx", "a backspace never crosses a line break");
        assert_eq!(terminal_text(b"a\x07b\x00c\td\x7f"), "abc\td");
        assert_eq!(terminal_text(&[b'o', 0xff, b'k']), "o\u{fffd}k");
    }
}
