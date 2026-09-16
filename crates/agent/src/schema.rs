//! Tool arguments checked against the tool's JSON Schema before they run, after pi's
//! `validateToolArguments`: values are first coerced the way a model tends to get them wrong
//! (numbers and booleans as strings, `null` for a field it meant to leave out), then validated,
//! and a failure is one message the model can act on.

use serde_json::{Map, Value};

/// Coerces and validates `arguments` against `schema`. `Ok` carries the arguments the tool
/// should run with; `Err` is the text of the error result.
pub fn validate_tool_arguments(tool_name: &str, schema: &Value, arguments: &Value) -> Result<Value, String> {
    let mut args = arguments.clone();
    normalize_optional_nulls(&mut args, schema);
    let coerced = coerce(args, schema);

    let validator = match jsonschema::validator_for(schema) {
        Ok(validator) => validator,
        // A schema the validator cannot compile is the tool's problem, not the model's.
        Err(_) => return Ok(coerced),
    };
    let errors: Vec<String> = validator.iter_errors(&coerced).map(|error| format!("  - {}: {error}", error_path(&error))).collect();
    if errors.is_empty() {
        return Ok(coerced);
    }
    let received = serde_json::to_string_pretty(arguments).unwrap_or_default();
    Err(format!("Validation failed for tool \"{tool_name}\":\n{}\n\nReceived arguments:\n{received}", errors.join("\n")))
}

/// The dotted path of an error; a missing property is reported at the property itself.
fn error_path(error: &jsonschema::ValidationError<'_>) -> String {
    let mut path: Vec<String> = error.instance_path().as_str().trim_start_matches('/').split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
    if let jsonschema::error::ValidationErrorKind::Required { property } = error.kind() {
        if let Some(name) = property.as_str() {
            path.push(name.to_string());
        }
    }
    if path.is_empty() {
        "root".into()
    } else {
        path.join(".")
    }
}

fn schema_types(schema: &Value) -> Vec<&str> {
    match &schema["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(types) => types.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn accepts_null(schema: &Value) -> bool {
    if schema_types(schema).contains(&"null") {
        return true;
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(options) = schema[key].as_array() {
            if options.iter().any(accepts_null) {
                return true;
            }
        }
    }
    // A schema with no type (or a reference) is not judged here.
    schema.get("type").is_none() && schema.get("anyOf").is_none() && schema.get("oneOf").is_none()
}

/// `null` for an optional property that does not take null means "not given": the key goes.
fn normalize_optional_nulls(value: &mut Value, schema: &Value) {
    match value {
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items").filter(|s| s.is_object()) {
                for item in items {
                    normalize_optional_nulls(item, item_schema);
                }
            }
        }
        Value::Object(object) => {
            let Some(properties) = schema["properties"].as_object() else { return };
            let required: Vec<&str> = schema["required"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            for (key, property_schema) in properties {
                let Some(current) = object.get_mut(key) else { continue };
                if current.is_null() && !required.contains(&key.as_str()) && !accepts_null(property_schema) {
                    object.remove(key);
                } else {
                    normalize_optional_nulls(current, property_schema);
                }
            }
        }
        _ => {}
    }
}

fn matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

fn coerce_primitive(value: Value, kind: &str) -> Value {
    match (kind, &value) {
        ("number", Value::Null) => Value::from(0),
        ("number", Value::String(s)) if !s.trim().is_empty() => s.trim().parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or(value),
        ("number", Value::Bool(b)) => Value::from(if *b { 1 } else { 0 }),
        ("integer", Value::Null) => Value::from(0),
        ("integer", Value::String(s)) if !s.trim().is_empty() => s.trim().parse::<i64>().map(Value::from).unwrap_or(value),
        ("integer", Value::Bool(b)) => Value::from(if *b { 1 } else { 0 }),
        ("boolean", Value::Null) => Value::Bool(false),
        ("boolean", Value::String(s)) if s == "true" => Value::Bool(true),
        ("boolean", Value::String(s)) if s == "false" => Value::Bool(false),
        ("boolean", Value::Number(n)) if n.as_i64() == Some(1) => Value::Bool(true),
        ("boolean", Value::Number(n)) if n.as_i64() == Some(0) => Value::Bool(false),
        ("string", Value::Null) => Value::String(String::new()),
        ("string", Value::Number(n)) => Value::String(n.to_string()),
        ("string", Value::Bool(b)) => Value::String(b.to_string()),
        ("null", Value::String(s)) if s.is_empty() => Value::Null,
        ("null", Value::Number(n)) if n.as_i64() == Some(0) => Value::Null,
        ("null", Value::Bool(false)) => Value::Null,
        _ => value,
    }
}

fn coerce(value: Value, schema: &Value) -> Value {
    let mut next = value;
    if let Some(all) = schema["allOf"].as_array() {
        for nested in all {
            next = coerce(next, nested);
        }
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(options) = schema[key].as_array() {
            next = coerce_union(next, options);
        }
    }

    let types = schema_types(schema);
    let already_matches = types.len() > 1 && types.iter().any(|t| matches_type(&next, t));
    if !types.is_empty() && !already_matches {
        for kind in &types {
            let candidate = coerce_primitive(next.clone(), kind);
            if candidate != next {
                next = candidate;
                break;
            }
        }
    }

    if types.contains(&"object") {
        if let Value::Object(object) = next {
            next = Value::Object(coerce_object(object, schema));
        }
    }
    if types.contains(&"array") {
        if let Value::Array(items) = next {
            next = Value::Array(coerce_array(items, schema));
        }
    }
    next
}

fn coerce_object(object: Map<String, Value>, schema: &Value) -> Map<String, Value> {
    let properties = schema["properties"].as_object();
    let additional = schema.get("additionalProperties").filter(|s| s.is_object());
    let mut out = Map::with_capacity(object.len());
    for (key, value) in object {
        let coerced = match properties.and_then(|p| p.get(&key)) {
            Some(property_schema) => coerce(value, property_schema),
            None => match additional {
                Some(schema) => coerce(value, schema),
                None => value,
            },
        };
        out.insert(key, coerced);
    }
    out
}

fn coerce_array(items: Vec<Value>, schema: &Value) -> Vec<Value> {
    match &schema["items"] {
        Value::Array(schemas) => items.into_iter().enumerate().map(|(i, item)| match schemas.get(i) {
            Some(s) => coerce(item, s),
            None => item,
        }).collect(),
        item_schema @ Value::Object(_) => items.into_iter().map(|item| coerce(item, item_schema)).collect(),
        _ => items,
    }
}

fn coerce_union(value: Value, options: &[Value]) -> Value {
    let valid = |candidate: &Value, schema: &Value| jsonschema::validator_for(schema).map(|v| v.is_valid(candidate)).unwrap_or(false);
    if options.iter().any(|schema| valid(&value, schema)) {
        return value;
    }
    for schema in options {
        let candidate = coerce(value.clone(), schema);
        if valid(&candidate, schema) {
            return candidate;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "limit": { "type": "integer" },
                "ratio": { "type": "number" },
                "all": { "type": "boolean" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "mode": { "type": ["string", "null"] },
            },
            "required": ["path"],
        })
    }

    #[test]
    fn coerces_what_models_get_wrong() {
        let args = json!({ "path": "a", "limit": "20", "ratio": "0.5", "all": "true", "tags": [1, true], "mode": null, "extra": null });
        let out = validate_tool_arguments("read", &schema(), &args).unwrap();
        assert_eq!(out, json!({ "path": "a", "limit": 20, "ratio": 0.5, "all": true, "tags": ["1", "true"], "mode": null, "extra": null }));
    }

    #[test]
    fn drops_null_for_an_optional_field_that_takes_none() {
        let out = validate_tool_arguments("read", &schema(), &json!({ "path": "a", "limit": null })).unwrap();
        assert_eq!(out, json!({ "path": "a" }));
    }

    #[test]
    fn reports_missing_and_wrong_fields_in_one_message() {
        let error = validate_tool_arguments("read", &schema(), &json!({ "limit": "lots" })).unwrap_err();
        assert!(error.starts_with("Validation failed for tool \"read\":\n"), "{error}");
        assert!(error.contains("  - path: "), "{error}");
        assert!(error.contains("  - limit: "), "{error}");
        assert!(error.contains("Received arguments:\n{\n  \"limit\": \"lots\"\n}"), "{error}");
    }

    #[test]
    fn a_schema_without_types_passes_anything_through() {
        let out = validate_tool_arguments("x", &json!({ "type": "object" }), &json!({ "anything": [1, "2"] })).unwrap();
        assert_eq!(out, json!({ "anything": [1, "2"] }));
    }
}
