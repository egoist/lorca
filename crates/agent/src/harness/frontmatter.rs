//! The `---` block at the top of a Markdown file: one `key: value` per line, quoted or bare
//! values, `true` / `false`. Enough for skills and prompt templates.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub fields: BTreeMap<String, String>,
    pub body: String,
}

impl Frontmatter {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str).filter(|v| !v.trim().is_empty())
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key).map(str::trim), Some("true" | "yes"))
    }
}

/// Splits a file into its frontmatter fields and body. A file without a `---` block is all
/// body. A field whose value is not a simple scalar is kept as its raw text.
pub fn parse_frontmatter(content: &str) -> Frontmatter {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let Some(rest) = normalized.strip_prefix("---") else { return Frontmatter { fields: BTreeMap::new(), body: normalized } };
    let Some(end) = rest.find("\n---") else { return Frontmatter { fields: BTreeMap::new(), body: normalized } };
    let block = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('-').trim().to_string();
    let mut fields = BTreeMap::new();
    let mut last_key: Option<String> = None;
    for line in block.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // A continuation line of a block scalar: appended to the last field.
            if let Some(key) = &last_key {
                let entry = fields.entry(key.clone()).or_insert_with(String::new);
                if !entry.is_empty() {
                    entry.push('\n');
                }
                entry.push_str(line.trim());
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim().to_string();
        let value = unquote(value.trim());
        // `|` and `>` start a block scalar on the following lines.
        let value = if value == "|" || value == ">" { String::new() } else { value };
        fields.insert(key.clone(), value);
        last_key = Some(key);
    }
    Frontmatter { fields, body }
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"') || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')) {
        let inner = &value[1..value.len() - 1];
        if bytes[0] == b'"' {
            return inner.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\");
        }
        return inner.replace("''", "'");
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields_quotes_and_block_scalars() {
        let text = "---\nname: my-skill\ndescription: \"Does things: well\"\ndisable-model-invocation: true\nnotes: |\n  first\n  second\n---\n\n# Body\ntext\n";
        let parsed = parse_frontmatter(text);
        assert_eq!(parsed.get("name"), Some("my-skill"));
        assert_eq!(parsed.get("description"), Some("Does things: well"));
        assert!(parsed.flag("disable-model-invocation"));
        assert_eq!(parsed.get("notes"), Some("first\nsecond"));
        assert_eq!(parsed.body, "# Body\ntext");
    }

    #[test]
    fn a_file_without_frontmatter_is_all_body() {
        let parsed = parse_frontmatter("just text\n---\nnot frontmatter");
        assert!(parsed.fields.is_empty());
        assert_eq!(parsed.body, "just text\n---\nnot frontmatter");
        assert_eq!(parse_frontmatter("---\nunterminated: yes\n").body, "---\nunterminated: yes\n");
    }
}
