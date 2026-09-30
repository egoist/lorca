//! TypeScript declarations of what a script can call, after pi's `declarations.ts`: a tool is
//! `name(args: T): Promise<R>;` with `T` and `R` rendered from its JSON Schemas, and its
//! description as a doc comment.

use std::collections::HashSet;

use serde_json::Value;

const INDENT: &str = "  ";
/// The longest rendered input type, in characters, before it becomes `unknown`.
pub const DEFAULT_INPUT_SCHEMA_MAX_CHARS: usize = 16_000;
/// Local `$ref` expansions per rendered schema, so shared definitions cannot blow up the output.
const MAX_REF_EXPANSIONS: usize = 32;

/// TypeScript types for MCP results, from the MCP `CallToolResult` schema, so `CallToolResult<T>`
/// declarations can refer to them.
pub const MCP_TYPESCRIPT_PREAMBLE: &str = r#"type Role = "user" | "assistant";
type MetaObject = Record<string, unknown>;
type Annotations = {
  audience?: Role[];
  priority?: number;
  lastModified?: string;
};
type TextResourceContents = {
  uri: string;
  mimeType?: string;
  text: string;
};
type BlobResourceContents = {
  uri: string;
  mimeType?: string;
  blob: string;
};
type TextContent = {
  type: "text";
  text: string;
  annotations?: Annotations;
};
type ImageContent = {
  type: "image";
  data: string;
  mimeType: string;
  annotations?: Annotations;
};
type AudioContent = {
  type: "audio";
  data: string;
  mimeType: string;
  annotations?: Annotations;
};
type ResourceLink = {
  type: "resource_link";
  name: string;
  title?: string;
  uri: string;
  description?: string;
  mimeType?: string;
  size?: number;
};
type EmbeddedResource = {
  type: "resource";
  resource: TextResourceContents | BlobResourceContents;
  annotations?: Annotations;
};
type ContentBlock =
  | TextContent
  | ImageContent
  | AudioContent
  | ResourceLink
  | EmbeddedResource;
type CallToolResult<TStructured = { [key: string]: unknown }> = {
  content: ContentBlock[];
  isError?: boolean;
  structuredContent?: TStructured;
};"#;

/// The identifier a script uses for a tool: characters that are not valid in a JavaScript
/// identifier become `_`. `github__create_issue` stays as it is, `my-tool` becomes `my_tool`.
pub fn to_identifier(name: &str) -> String {
    let mut identifier = String::with_capacity(name.len());
    for c in name.chars() {
        let valid = if identifier.is_empty() { c.is_ascii_alphabetic() || c == '_' || c == '$' } else { c.is_ascii_alphanumeric() || c == '_' || c == '$' };
        identifier.push(if valid { c } else { '_' });
    }
    if identifier.is_empty() {
        "_".into()
    } else {
        identifier
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A tool as a member of the `tools` object: `id(args: T): Promise<R>;`. An input type longer
/// than `input_max_chars` renders as `unknown`. An output schema shaped like MCP's
/// `CallToolResult` renders as `Promise<CallToolResult<T>>`, which needs
/// [`MCP_TYPESCRIPT_PREAMBLE`].
pub fn tool_signature(name: &str, input: Option<&Value>, output: Option<&Value>, input_max_chars: usize) -> String {
    let input = input.map(|schema| schema_to_type(schema, Some(input_max_chars))).unwrap_or_else(|| "unknown".into());
    format!("{}(args: {input}): Promise<{}>;", to_identifier(name), output_type(output))
}

/// A tool's sample: its description followed by its declaration, as search results and
/// `describeTool()` give it.
pub fn tool_sample(name: &str, description: &str, input: Option<&Value>, output: Option<&Value>) -> String {
    let declaration = format!("declare const tools: {{ {} }};", tool_signature(name, input, output, DEFAULT_INPUT_SCHEMA_MAX_CHARS));
    format!("{}\n\ncodemode tool declaration:\n```ts\n{declaration}\n```", description.trim())
}

/// The `structuredContent` schema of an MCP `CallToolResult` output schema (a `content` array of
/// objects, and a boolean `isError`), `true` when it declares none, or `None` when the schema
/// is not a `CallToolResult`.
pub fn mcp_structured_content_schema(schema: Option<&Value>) -> Option<Value> {
    let properties = schema?.get("properties")?.as_object()?;
    let content = properties.get("content")?;
    if content.get("type")?.as_str()? != "array" || content.get("items")?.get("type")?.as_str()? != "object" {
        return None;
    }
    if properties.get("isError")?.get("type")?.as_str()? != "boolean" {
        return None;
    }
    Some(match properties.get("structuredContent") {
        Some(structured @ (Value::Object(_) | Value::Bool(_))) => structured.clone(),
        _ => Value::Bool(true),
    })
}

fn output_type(schema: Option<&Value>) -> String {
    if let Some(structured) = mcp_structured_content_schema(schema) {
        let rendered = schema_to_type(&structured, None);
        return if rendered == "unknown" { "CallToolResult".into() } else { format!("CallToolResult<{rendered}>") };
    }
    schema.map(|schema| schema_to_type(schema, None)).unwrap_or_else(|| "unknown".into())
}

/// A doc comment for `description` at `indent`, or nothing for an empty one.
pub fn doc_comment(description: &str, indent: &str) -> String {
    let text = description.trim();
    if text.is_empty() {
        return String::new();
    }
    let text = text.replace("*/", "*\\/");
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() == 1 {
        return format!("{indent}/** {} */\n", lines[0]);
    }
    let mut out = format!("{indent}/**\n");
    for line in lines {
        if line.is_empty() {
            out.push_str(&format!("{indent} *\n"));
        } else {
            out.push_str(&format!("{indent} * {line}\n"));
        }
    }
    out.push_str(&format!("{indent} */\n"));
    out
}

/// A JSON Schema as a TypeScript type expression: an object on one line
/// (`{ a: string; b?: number; }`) with its properties sorted by name, or one property per line
/// with `//` comments when a property has a description; `Array<T>` for arrays. Local
/// references (`#/$defs/...`, `#/definitions/...`) resolve against `schema`; recursive and
/// remote ones render as `unknown`, and so does a result longer than `max_chars`.
pub fn schema_to_type(schema: &Value, max_chars: Option<usize>) -> String {
    let mut context = SchemaContext { root: schema, resolving: HashSet::new(), expansions: 0 };
    let rendered = to_type(schema, &mut context);
    match max_chars {
        Some(max) if rendered.chars().count() > max => "unknown".into(),
        _ => rendered,
    }
}

struct SchemaContext<'a> {
    root: &'a Value,
    /// References being expanded on the current path, to stop at recursive types.
    resolving: HashSet<String>,
    expansions: usize,
}

fn resolve_ref<'a>(reference: &str, root: &'a Value) -> Option<&'a Value> {
    if reference != "#" && !reference.starts_with("#/") {
        return None;
    }
    let mut current = root;
    for segment in reference.trim_start_matches('#').split('/').filter(|segment| !segment.is_empty()) {
        let key = percent_decode(segment).replace("~1", "/").replace("~0", "~");
        current = current.as_object()?.get(&key)?;
    }
    matches!(current, Value::Object(_) | Value::Bool(_)).then_some(current)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |byte: u8| (byte as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 3 <= bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn json_literal(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "unknown".into())
}

fn union(types: Vec<String>) -> String {
    let mut unique: Vec<String> = Vec::new();
    for rendered in types {
        if !unique.contains(&rendered) {
            unique.push(rendered);
        }
    }
    if unique.iter().any(|rendered| rendered == "unknown") {
        return "unknown".into();
    }
    if unique.is_empty() {
        "never".into()
    } else {
        unique.join(" | ")
    }
}

fn to_type(schema: &Value, context: &mut SchemaContext<'_>) -> String {
    let object = match schema {
        Value::Bool(true) => return "unknown".into(),
        Value::Bool(false) => return "never".into(),
        Value::Object(object) => object,
        _ => return "unknown".into(),
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if context.resolving.contains(reference) || context.expansions >= MAX_REF_EXPANSIONS {
            return "unknown".into();
        }
        let Some(target) = resolve_ref(reference, context.root) else { return "unknown".into() };
        context.expansions += 1;
        context.resolving.insert(reference.to_string());
        let rendered = to_type(target, context);
        context.resolving.remove(reference);
        return rendered;
    }
    if let Some(constant) = object.get("const") {
        return json_literal(constant);
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return union(values.iter().map(json_literal).collect());
    }
    let variants = object.get("anyOf").and_then(Value::as_array).or_else(|| object.get("oneOf").and_then(Value::as_array));
    if let Some(variants) = variants {
        return union(variants.iter().map(|variant| to_type(variant, context)).collect());
    }
    if let Some(parts) = object.get("allOf").and_then(Value::as_array) {
        let parts: Vec<String> = parts.iter().map(|part| to_type(part, context)).filter(|part| part != "unknown").collect();
        if parts.is_empty() {
            return "unknown".into();
        }
        return parts.iter().map(|part| if part.contains(" | ") { format!("({part})") } else { part.clone() }).collect::<Vec<_>>().join(" & ");
    }

    match object.get("type") {
        Some(Value::Array(types)) => {
            let types: Vec<String> = types
                .iter()
                .map(|entry| {
                    let mut single = object.clone();
                    single.insert("type".into(), entry.clone());
                    to_type(&Value::Object(single), context)
                })
                .collect();
            union(types)
        }
        Some(Value::String(kind)) => match kind.as_str() {
            "string" => "string".into(),
            "number" | "integer" => "number".into(),
            "boolean" => "boolean".into(),
            "null" => "null".into(),
            "array" => array_type(object, context),
            "object" => object_type(object, context),
            _ => "unknown".into(),
        },
        None => {
            if object.contains_key("properties") || object.contains_key("additionalProperties") || object.contains_key("required") {
                object_type(object, context)
            } else if object.contains_key("items") || object.contains_key("prefixItems") {
                array_type(object, context)
            } else {
                "unknown".into()
            }
        }
        Some(_) => "unknown".into(),
    }
}

fn array_type(schema: &serde_json::Map<String, Value>, context: &mut SchemaContext<'_>) -> String {
    if let Some(items) = schema.get("items").filter(|items| !items.is_array()) {
        return format!("Array<{}>", to_type(items, context));
    }
    let tuple = schema.get("prefixItems").and_then(Value::as_array).or_else(|| schema.get("items").and_then(Value::as_array));
    match tuple {
        Some(items) if !items.is_empty() => format!("[{}]", items.iter().map(|item| to_type(item, context)).collect::<Vec<_>>().join(", ")),
        _ => "unknown[]".into(),
    }
}

fn description_of(property: &Value) -> &str {
    property.get("description").and_then(Value::as_str).map(str::trim).unwrap_or("")
}

fn property_key(name: &str) -> String {
    if is_identifier(name) {
        name.to_string()
    } else {
        serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\""))
    }
}

fn object_type(schema: &serde_json::Map<String, Value>, context: &mut SchemaContext<'_>) -> String {
    let empty = serde_json::Map::new();
    let properties = schema.get("properties").and_then(Value::as_object).unwrap_or(&empty);
    let required: HashSet<&str> = schema.get("required").and_then(Value::as_array).map(|names| names.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut names: Vec<&String> = properties.keys().collect();
    names.sort();
    let mut members: Vec<String> = names
        .iter()
        .map(|name| {
            let optional = if required.contains(name.as_str()) { "" } else { "?" };
            format!("{}{optional}: {};", property_key(name), to_type(&properties[name.as_str()], context))
        })
        .collect();
    match schema.get("additionalProperties") {
        Some(Value::Bool(false)) => {}
        Some(additional) => {
            let rendered = if additional == &Value::Bool(true) { "unknown".to_string() } else { to_type(additional, context) };
            members.push(format!("[key: string]: {rendered};"));
        }
        None if names.is_empty() => members.push("[key: string]: unknown;".into()),
        None => {}
    }
    if members.is_empty() {
        return "{}".into();
    }
    if !names.iter().any(|name| !description_of(&properties[name.as_str()]).is_empty()) {
        return format!("{{ {} }}", members.join(" "));
    }
    let mut lines = vec!["{".to_string()];
    for (index, name) in names.iter().enumerate() {
        for line in description_of(&properties[name.as_str()]).lines() {
            if !line.trim().is_empty() {
                lines.push(format!("{INDENT}// {}", line.trim()));
            }
        }
        lines.push(format!("{INDENT}{}", members[index].replace('\n', &format!("\n{INDENT}"))));
    }
    for member in &members[names.len()..] {
        lines.push(format!("{INDENT}{member}"));
    }
    lines.push("}".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schemas_render_as_typescript() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "limit": { "type": "integer" },
                "mode": { "enum": ["a", "b"] },
                "tags": { "type": "array", "items": { "type": "string" } },
                "when": { "anyOf": [{ "type": "string" }, { "type": "null" }] }
            },
            "required": ["path"]
        });
        assert_eq!(
            schema_to_type(&schema, None),
            "{ limit?: number; mode?: \"a\" | \"b\"; path: string; tags?: Array<string>; when?: string | null; }"
        );
        assert_eq!(schema_to_type(&json!({ "type": "object" }), None), "{ [key: string]: unknown; }");
        assert_eq!(schema_to_type(&json!({ "type": ["string", "number"] }), None), "string | number");
        assert_eq!(schema_to_type(&json!({ "prefixItems": [{ "type": "string" }, { "const": 1 }] }), None), "[string, 1]");
        assert_eq!(schema_to_type(&json!({ "properties": { "a-b": { "type": "boolean" } }, "additionalProperties": false }), None), "{ \"a-b\"?: boolean; }");
    }

    #[test]
    fn described_properties_get_comments_and_refs_resolve() {
        let schema = json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner" },
                "node": { "$ref": "#/$defs/node" }
            },
            "required": ["owner", "node"],
            "$defs": { "node": { "type": "object", "properties": { "next": { "$ref": "#/$defs/node" } } } }
        });
        assert_eq!(schema_to_type(&schema, None), "{\n  node: { next?: unknown; };\n  // Repository owner\n  owner: string;\n}");
        assert_eq!(schema_to_type(&json!({ "type": "string", "maxLength": 3 }), Some(3)), "unknown");
    }

    #[test]
    fn signatures_name_mcp_results_and_identifiers() {
        let output = json!({
            "type": "object",
            "properties": {
                "content": { "type": "array", "items": { "type": "object" } },
                "structuredContent": { "type": "object", "properties": { "id": { "type": "number" } }, "required": ["id"] },
                "isError": { "type": "boolean" }
            }
        });
        assert_eq!(
            tool_signature("github__create-issue", Some(&json!({ "type": "object", "properties": {} })), Some(&output), 100),
            "github__create_issue(args: { [key: string]: unknown; }): Promise<CallToolResult<{ id: number; }>>;"
        );
        assert_eq!(tool_signature("read", None, Some(&json!({ "type": "string" })), 100), "read(args: unknown): Promise<string>;");
        assert_eq!(to_identifier("9lives"), "_lives");
        assert_eq!(doc_comment("one\n\ntwo */", ""), "/**\n * one\n *\n * two *\\/\n */\n");
    }
}
