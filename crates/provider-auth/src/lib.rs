//! Provider OAuth tokens and PKCE loopback flows. Every Device can connect the account; a
//! Runner also uses these types to refresh the tokens before model calls.

use serde_json::Value;

pub mod chatgpt;
pub mod grok;

/// Why a token endpoint refused: OAuth's `error_description` or `error`, else the
/// `{"error": {"message", "code"}}` OpenAI answers outside OAuth, as when it does not serve the
/// country a request comes from (403 `unsupported_country_region_territory`).
pub(crate) fn error_message(body: &Value) -> Option<&str> {
    body["error_description"]
        .as_str()
        .or(body["error"].as_str())
        .or(body["error"]["message"].as_str())
        .or(body["error"]["code"].as_str())
        .or(body["message"].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_oauth_and_api_errors() {
        assert_eq!(error_message(&json!({ "error": "invalid_grant", "error_description": "Code expired" })), Some("Code expired"));
        assert_eq!(error_message(&json!({ "error": "invalid_grant" })), Some("invalid_grant"));
        let region = json!({ "error": { "code": "unsupported_country_region_territory", "message": "Country, region, or territory not supported", "type": "request_forbidden" } });
        assert_eq!(error_message(&region), Some("Country, region, or territory not supported"));
        assert_eq!(error_message(&Value::Null), None);
    }
}
