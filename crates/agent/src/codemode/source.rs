//! A script's source: JavaScript, optionally preceded by one options line, after pi's
//! `source.ts`.
//!
//! ```js
//! // @options: {"max_output_tokens": 2000, "timeout_ms": 30000}
//! const { issues } = await tools.linear__list_issues({ state: "open" });
//! return issues.length;
//! ```

use serde_json::Value;

pub const OPTIONS_PREFIX: &str = "// @options:";
const SUPPORTED_FIELDS: &str = "`max_output_tokens` and `timeout_ms`";
/// The longest deadline a timer can take, which bounds `timeout_ms`.
const MAX_TIMEOUT_MS: u64 = 2_147_483_647;

/// What the options line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceOptions {
    /// The token budget for the script's output.
    pub max_output_tokens: Option<u64>,
    /// A hard deadline for the whole script in milliseconds, tool calls included.
    pub timeout_ms: Option<u64>,
}

/// A script with its options line replaced by an empty line, so line numbers are unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSource {
    pub code: String,
    pub options: SourceOptions,
}

/// Splits an optional first-line `// @options: {...}` from the script. The error is what the
/// model reads: empty input, invalid options, or an options line with no code after it.
pub fn parse_source(input: &str) -> Result<ParsedSource, String> {
    if input.trim().is_empty() {
        return Err(
            "Expected JavaScript source text (non-empty). Provide JS only, optionally with a first line `// @options: {\"max_output_tokens\": 1000}`."
                .into(),
        );
    }
    let (first_line, rest) = match input.find('\n') {
        Some(newline) => (&input[..newline], Some(&input[newline..])),
        None => (input, None),
    };
    let trimmed = first_line.trim_end_matches('\r').trim_start();
    let Some(directive) = trimmed.strip_prefix(OPTIONS_PREFIX) else {
        return Ok(ParsedSource { code: input.to_string(), options: SourceOptions::default() });
    };
    let code = rest.unwrap_or("");
    if code.trim().is_empty() {
        return Err("The @options line must be followed by JavaScript source on the next lines".into());
    }
    Ok(ParsedSource { code: code.to_string(), options: parse_options(directive.trim())? })
}

fn parse_options(directive: &str) -> Result<SourceOptions, String> {
    let value: Value = serde_json::from_str(directive).map_err(|error| format!("@options must be valid JSON with supported fields {SUPPORTED_FIELDS}: {error}"))?;
    let Value::Object(fields) = value else {
        return Err(format!("@options must be a JSON object with supported fields {SUPPORTED_FIELDS}"));
    };
    if let Some(key) = fields.keys().find(|key| !["max_output_tokens", "timeout_ms"].contains(&key.as_str())) {
        return Err(format!("@options only supports {SUPPORTED_FIELDS}; got `{key}`"));
    }
    let mut options = SourceOptions::default();
    if let Some(value) = fields.get("max_output_tokens") {
        options.max_output_tokens = Some(value.as_u64().ok_or("@options field `max_output_tokens` must be a non-negative integer")?);
    }
    if let Some(value) = fields.get("timeout_ms") {
        let timeout = value.as_u64().filter(|timeout| (1..=MAX_TIMEOUT_MS).contains(timeout));
        options.timeout_ms = Some(timeout.ok_or_else(|| format!("@options field `timeout_ms` must be a positive integer up to {MAX_TIMEOUT_MS}"))?);
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_line_splits_off_and_keeps_line_numbers() {
        let parsed = parse_source("// @options: {\"max_output_tokens\": 20, \"timeout_ms\": 500}\nreturn 1;").unwrap();
        assert_eq!(parsed.code, "\nreturn 1;");
        assert_eq!(parsed.options, SourceOptions { max_output_tokens: Some(20), timeout_ms: Some(500) });
        assert_eq!(parse_source("return 2;").unwrap().code, "return 2;");
        assert!(parse_source("  ").is_err());
        assert!(parse_source("// @options: {\"x\": 1}\nreturn 1;").unwrap_err().contains("`x`"));
        assert!(parse_source("// @options: {\"timeout_ms\": 0}\nreturn 1;").is_err());
        assert!(parse_source("// @options: {}").is_err());
    }
}
