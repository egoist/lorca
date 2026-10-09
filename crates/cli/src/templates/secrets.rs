//! Exact local credential values supplement pattern scrubbing. They are used only to redact
//! selected text; no credential record or value is returned, logged, or put in a template.

use crate::app::App;
use serde_json::Value;
use std::collections::BTreeSet;

pub fn local(app: &App) -> Vec<String> {
    let mut secrets = BTreeSet::new();
    collect(
        &serde_json::to_value(&*app.credentials.lock().unwrap()).unwrap_or(Value::Null),
        &mut secrets,
    );
    let plugins = app.plugins.lock().unwrap();
    for plugin in plugins.installed() {
        collect(
            &serde_json::to_value(&plugin.manifest).unwrap_or(Value::Null),
            &mut secrets,
        );
        for variable in &plugin.manifest.variables {
            if variable.secret {
                if let Some(value) = plugins
                    .secret(&plugin.manifest.id, &variable.name)
                    .and_then(|v| v.as_str().map(str::to_string))
                {
                    if !value.is_empty() {
                        secrets.insert(value);
                    }
                }
            }
        }
        for server in plugin.manifest.servers.keys() {
            if let Some(value) = plugins.secret(&plugin.manifest.id, &format!("oauth:{server}")) {
                collect(&value, &mut secrets);
            }
        }
    }
    let mut secrets: Vec<_> = secrets.into_iter().collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secrets
}

fn collect(value: &Value, secrets: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let key = key.to_ascii_lowercase();
                if key == "api_key"
                    || key == "x-api-key"
                    || key == "authorization"
                    || key == "cookie"
                    || key == "token"
                    || key.ends_with("_token")
                    || key.ends_with("_secret")
                    || key == "password"
                    || key == "session_key"
                {
                    if let Some(text) = value
                        .as_str()
                        .filter(|text| !text.is_empty() && !text.contains("${"))
                    {
                        secrets.insert(text.to_string());
                        if text
                            .get(..7)
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer "))
                            && text.len() > 7
                        {
                            secrets.insert(text[7..].trim().into());
                        }
                    }
                }
                collect(value, secrets);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect(value, secrets);
            }
        }
        _ => {}
    }
}

pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_string();
    for secret in secrets {
        text = text.replace(secret, "«redacted credential»");
    }
    text
}
