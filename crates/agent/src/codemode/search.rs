//! Tool search for `searchTools()`: Okapi BM25 over each tool's name, description, schema text,
//! and namespace, after pi's tool search.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// How many matches `searchTools()` returns unless the script asks for another number.
pub const DEFAULT_SEARCH_LIMIT: usize = 8;

const STOP_WORDS: &[&str] = &["a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on", "or", "that", "the", "this", "to", "with"];

/// A naive singular, so `issues` matches `issue` and `searches` matches `search`.
fn stem(term: &str) -> String {
    let n = term.len();
    if n > 4 && term.ends_with("ies") {
        return format!("{}y", &term[..n - 3]);
    }
    if n > 4 && ["ches", "shes", "sses", "xes", "zes"].iter().any(|suffix| term.ends_with(suffix)) {
        return term[..n - 2].to_string();
    }
    if n > 3 && term.ends_with('s') && !term.ends_with("ss") {
        return term[..n - 1].to_string();
    }
    term.to_string()
}

/// Lowercase terms split at camelCase boundaries and anything not alphanumeric, without stop
/// words.
pub fn tokenize(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut spaced = String::with_capacity(text.len() + 8);
    for (index, &c) in chars.iter().enumerate() {
        if index > 0 && c.is_ascii_uppercase() {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() || (previous.is_ascii_uppercase() && next_lower) {
                spaced.push(' ');
            }
        }
        spaced.push(c);
    }
    spaced
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|term| !term.is_empty() && !STOP_WORDS.contains(term))
        .map(stem)
        .collect()
}

/// Schema descriptions and property names, recursively.
fn schema_text(schema: &Value, parts: &mut Vec<String>) {
    let Some(object) = schema.as_object() else { return };
    if let Some(description) = object.get("description").and_then(Value::as_str) {
        parts.push(description.to_string());
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            parts.push(name.clone());
            schema_text(property, parts);
        }
    }
    if let Some(items) = object.get("items") {
        schema_text(items, parts);
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = object.get(key).and_then(Value::as_array) {
            for variant in variants {
                schema_text(variant, parts);
            }
        }
    }
}

/// A tool's search text: its name, the name with `_` as spaces, its description, its schema's
/// descriptions and property names, and its namespace.
pub fn document(name: &str, description: &str, parameters: &Value, namespace: Option<(&str, &str)>) -> String {
    let mut parts = vec![name.to_string(), name.replace('_', " "), description.to_string()];
    schema_text(parameters, &mut parts);
    if let Some((namespace, about)) = namespace {
        parts.push(namespace.to_string());
        parts.push(about.to_string());
    }
    parts.retain(|part| !part.trim().is_empty());
    parts.join(" ")
}

/// The indexes of the documents that match `query`, best first, at most `limit`. Ties keep
/// document order.
pub fn rank(query: &str, documents: &[String], limit: usize) -> Vec<usize> {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let mut seen = HashSet::new();
    let terms: Vec<String> = tokenize(query).into_iter().filter(|term| seen.insert(term.clone())).collect();
    if terms.is_empty() || documents.is_empty() || limit == 0 {
        return Vec::new();
    }
    let counts: Vec<HashMap<String, usize>> = documents
        .iter()
        .map(|document| {
            let mut counts = HashMap::new();
            for term in tokenize(document) {
                *counts.entry(term).or_insert(0) += 1;
            }
            counts
        })
        .collect();
    let lengths: Vec<f64> = counts.iter().map(|counts| counts.values().sum::<usize>() as f64).collect();
    let average = (lengths.iter().sum::<f64>() / documents.len() as f64).max(1.0);
    let total = documents.len() as f64;
    let idf: Vec<f64> = terms
        .iter()
        .map(|term| {
            let frequency = counts.iter().filter(|counts| counts.contains_key(term)).count() as f64;
            (1.0 + (total - frequency + 0.5) / (frequency + 0.5)).ln()
        })
        .collect();
    let mut matches: Vec<(usize, f64)> = Vec::new();
    for (index, document_counts) in counts.iter().enumerate() {
        let mut score = 0.0;
        for (term_index, term) in terms.iter().enumerate() {
            let Some(&count) = document_counts.get(term) else { continue };
            let count = count as f64;
            let norm = K1 * (1.0 - B + B * lengths[index] / average);
            score += idf[term_index] * (count * (K1 + 1.0)) / (count + norm);
        }
        if score > 0.0 {
            matches.push((index, score));
        }
    }
    matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    matches.into_iter().take(limit).map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tokens_split_camel_case_and_stem() {
        assert_eq!(tokenize("listIssues for the HTTPServer"), vec!["list", "issue", "http", "server"]);
        assert_eq!(tokenize("searches queries"), vec!["search", "query"]);
    }

    #[test]
    fn ranking_prefers_the_named_capability() {
        let documents = vec![
            document("linear__list_issues", "List issues in a team", &json!({ "properties": { "team": { "type": "string" } } }), Some(("linear", "Linear"))),
            document("linear__create_comment", "Comment on an issue", &json!({}), Some(("linear", "Linear"))),
            document("read", "Read a file", &json!({ "properties": { "path": {} } }), None),
        ];
        assert_eq!(rank("list open issues", &documents, 8), vec![0, 1]);
        assert_eq!(rank("read file path", &documents, 1), vec![2]);
        assert!(rank("the", &documents, 8).is_empty());
    }
}
