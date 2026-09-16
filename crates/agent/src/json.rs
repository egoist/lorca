//! Tool-argument JSON as models emit it: repaired when a string carries raw control characters
//! or a bad escape, salvaged when the output stopped mid-value. After pi's `json-parse`.

use serde_json::{Map, Value};

/// Fixes the string literals of otherwise well-formed JSON: raw control characters are escaped
/// and a backslash before something that is not an escape is doubled.
pub fn repair_json(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut out = String::with_capacity(json.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !in_string {
            out.push(c);
            if c == '"' {
                in_string = true;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            out.push(c);
            in_string = false;
            i += 1;
            continue;
        }
        if c == '\\' {
            let Some(&next) = chars.get(i + 1) else {
                out.push_str("\\\\");
                i += 1;
                continue;
            };
            if next == 'u' {
                let digits: String = chars[i + 2..].iter().take(4).collect();
                if digits.len() == 4 && digits.chars().all(|d| d.is_ascii_hexdigit()) {
                    out.push_str("\\u");
                    out.push_str(&digits);
                    i += 6;
                    continue;
                }
            }
            if matches!(next, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') {
                out.push('\\');
                out.push(next);
                i += 2;
                continue;
            }
            out.push_str("\\\\");
            i += 1;
            continue;
        }
        if (c as u32) < 0x20 {
            match c {
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                other => out.push_str(&format!("\\u{:04x}", other as u32)),
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Parses JSON, repairing its string literals when the first attempt fails.
pub fn parse_json_with_repair(json: &str) -> Result<Value, serde_json::Error> {
    match serde_json::from_str(json) {
        Ok(value) => Ok(value),
        Err(error) => {
            let repaired = repair_json(json);
            if repaired != json {
                serde_json::from_str(&repaired)
            } else {
                Err(error)
            }
        }
    }
}

/// Streamed tool arguments so far, as an object. Complete JSON parses as is; JSON cut off
/// mid-value keeps every complete member and the partial string or number it stopped in.
/// Anything else, and text that is not an object, is `{}`.
pub fn parse_streaming_json(partial: &str) -> Value {
    let trimmed = partial.trim();
    if trimmed.is_empty() {
        return Value::Object(Map::new());
    }
    let parsed = parse_json_with_repair(trimmed)
        .ok()
        .or_else(|| parse_partial_json(trimmed))
        .or_else(|| parse_partial_json(&repair_json(trimmed)));
    match parsed {
        Some(value @ Value::Object(_)) => value,
        _ => Value::Object(Map::new()),
    }
}

/// Best-effort parse of JSON that may end anywhere: open strings, arrays, and objects are
/// closed, a key without a value is dropped, and a value that cannot be completed is left out.
pub fn parse_partial_json(json: &str) -> Option<Value> {
    let chars: Vec<char> = json.chars().collect();
    let mut parser = Partial { chars: &chars, pos: 0 };
    parser.skip_ws();
    let value = parser.value()?;
    Some(value)
}

struct Partial<'a> {
    chars: &'a [char],
    pos: usize,
}

impl Partial<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    /// A value, or `None` when there is not enough to tell what it is.
    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            '{' => Some(self.object()),
            '[' => Some(self.array()),
            '"' => Some(Value::String(self.string())),
            't' => self.literal("true", Value::Bool(true)),
            'f' => self.literal("false", Value::Bool(false)),
            'n' => self.literal("null", Value::Null),
            c if c == '-' || c.is_ascii_digit() => self.number(),
            _ => None,
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Option<Value> {
        let rest: String = self.chars[self.pos..].iter().take(word.len()).collect();
        if rest == word {
            self.pos += word.len();
            return Some(value);
        }
        // A prefix of the word at the end of the input: cut off, nothing to keep.
        if word.starts_with(&rest) && self.pos + rest.len() == self.chars.len() {
            self.pos = self.chars.len();
        }
        None
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')) {
            self.pos += 1;
        }
        let mut text: String = self.chars[start..self.pos].iter().collect();
        // Trim a trailing sign, dot, or exponent marker the model had not finished.
        while text.ends_with(['-', '+', '.', 'e', 'E']) {
            text.pop();
        }
        serde_json::from_str::<serde_json::Number>(&text).ok().map(Value::Number)
    }

    /// A string; the closing quote may be missing. An escape cut off at the end is dropped.
    fn string(&mut self) -> String {
        self.pos += 1;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '"' => return out,
                '\\' => {
                    let Some(next) = self.peek() else { break };
                    self.pos += 1;
                    match next {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let digits: String = self.chars[self.pos..].iter().take(4).collect();
                            if digits.len() < 4 {
                                self.pos = self.chars.len();
                                break;
                            }
                            self.pos += 4;
                            if let Some(ch) = u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32) {
                                out.push(ch);
                            }
                        }
                        other => {
                            out.push('\\');
                            out.push(other);
                        }
                    }
                }
                other => out.push(other),
            }
        }
        out
    }

    fn array(&mut self) -> Value {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                Some(',') => {
                    self.pos += 1;
                    continue;
                }
                Some(_) => match self.value() {
                    Some(value) => items.push(value),
                    None => {
                        self.pos = self.chars.len();
                        break;
                    }
                },
            }
        }
        Value::Array(items)
    }

    fn object(&mut self) -> Value {
        self.pos += 1;
        let mut members = Map::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                Some(',') => {
                    self.pos += 1;
                    continue;
                }
                Some('"') => {
                    let key = self.string();
                    self.skip_ws();
                    if self.peek() != Some(':') {
                        // The key was cut off, or its colon is missing: nothing to keep.
                        self.pos = self.chars.len();
                        break;
                    }
                    self.pos += 1;
                    self.skip_ws();
                    match self.value() {
                        Some(value) => {
                            members.insert(key, value);
                        }
                        None => {
                            self.pos = self.chars.len();
                            break;
                        }
                    }
                }
                Some(_) => {
                    self.pos = self.chars.len();
                    break;
                }
            }
        }
        Value::Object(members)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repairs_control_characters_and_bad_escapes() {
        let raw = "{\"text\": \"line1\nline2\", \"path\": \"C:\\Users\\x\"}";
        assert!(serde_json::from_str::<Value>(raw).is_err());
        let parsed = parse_json_with_repair(raw).unwrap();
        assert_eq!(parsed["text"], "line1\nline2");
        assert_eq!(parsed["path"], "C:\\Users\\x");
        // Valid escapes and unicode are left alone.
        assert_eq!(repair_json(r#"{"a":"\n\u00e9\\"}"#), r#"{"a":"\n\u00e9\\"}"#);
    }

    #[test]
    fn salvages_json_cut_off_anywhere() {
        assert_eq!(parse_partial_json(r#"{"path": "a/b", "content": "hel"#), Some(json!({ "path": "a/b", "content": "hel" })));
        assert_eq!(parse_partial_json(r#"{"path": "a/b", "cont"#), Some(json!({ "path": "a/b" })));
        assert_eq!(parse_partial_json(r#"{"path": "a/b", "content":"#), Some(json!({ "path": "a/b" })));
        assert_eq!(parse_partial_json(r#"{"n": 12, "list": [1, 2, {"k": tr"#), Some(json!({ "n": 12, "list": [1, 2, {}] })));
        assert_eq!(parse_partial_json(r#"{"n": -"#), Some(json!({})));
        assert_eq!(parse_partial_json(r#"{"s": "a\"#), Some(json!({ "s": "a" })));
        assert_eq!(parse_partial_json(r#"{"s": "caf\u00"#), Some(json!({ "s": "caf" })));
        assert_eq!(parse_partial_json("["), Some(json!([])));
        assert_eq!(parse_partial_json("x"), None);
    }

    #[test]
    fn streaming_json_is_always_an_object() {
        assert_eq!(parse_streaming_json(""), json!({}));
        assert_eq!(parse_streaming_json("   "), json!({}));
        assert_eq!(parse_streaming_json("[1]"), json!({}));
        assert_eq!(parse_streaming_json("not json"), json!({}));
        assert_eq!(parse_streaming_json(r#"{"a": 1}"#), json!({ "a": 1 }));
        assert_eq!(parse_streaming_json("{\"a\": \"x\ny\", \"b\": [1,"), json!({ "a": "x\ny", "b": [1] }));
    }
}
