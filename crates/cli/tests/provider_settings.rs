use lorca::{api, app::App, config::Config, credentials::ApiKeyCredential};
use serde_json::json;

struct Home(std::path::PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn provider_settings_reads_only_the_requested_api_key() {
    let home = Home(std::env::temp_dir().join(format!("lorca-provider-settings-{}", uuid::Uuid::new_v4())));
    let app = App::load(Config { home: home.0.clone(), port: 0 }).unwrap();
    let kinds = ["deepseek", "anthropic", "opencode", "opencode-go"];

    for kind in kinds {
        let result = api::dispatch(&app, "providers.api_key", json!({ "kind": kind })).await.unwrap();
        assert_eq!(result, json!({ "api_key": null, "base_url": null }));
    }

    {
        let mut credentials = app.credentials.lock().unwrap();
        credentials.deepseek = Some(ApiKeyCredential {
            api_key: "sk-test-deepseek-secret".into(),
            base_url: Some("https://deepseek.example.test/v1".into()),
            connected_at: 1,
        });
        credentials.anthropic = Some(ApiKeyCredential { api_key: "sk-test-anthropic-secret".into(), base_url: None, connected_at: 1 });
        credentials.opencode = Some(ApiKeyCredential { api_key: "sk-test-opencode-secret".into(), base_url: None, connected_at: 1 });
        credentials.opencode_go = Some(ApiKeyCredential { api_key: "sk-test-opencode-go-secret".into(), base_url: None, connected_at: 1 });
    }

    for kind in kinds {
        let result = api::dispatch(&app, "providers.api_key", json!({ "kind": kind })).await.unwrap();
        assert_eq!(result["api_key"], format!("sk-test-{kind}-secret"));
        assert_eq!(result["base_url"], if kind == "deepseek" { json!("https://deepseek.example.test/v1") } else { json!(null) });
        assert_eq!(result.as_object().unwrap().len(), 2);
    }

    // Normal account updates keep the full keys out of the UI's shared model.
    let snapshot = app.snapshot().to_string();
    let statuses = serde_json::to_string(&app.credentials.lock().unwrap().statuses()).unwrap();
    for kind in kinds {
        let key = format!("sk-test-{kind}-secret");
        assert!(!snapshot.contains(&key));
        assert!(!statuses.contains(&key));
    }
    for kind in ["chatgpt", "grok", "unknown"] {
        assert!(api::dispatch(&app, "providers.api_key", json!({ "kind": kind })).await.is_err());
    }
    assert!(api::dispatch(&app, "providers.api_key", json!({})).await.is_err());
}

#[tokio::test]
async fn auto_review_reviews_with_a_connected_providers_model() {
    let home = Home(std::env::temp_dir().join(format!("lorca-auto-review-model-{}", uuid::Uuid::new_v4())));
    let app = App::load(Config { home: home.0.clone(), port: 0 }).unwrap();
    let set = |params| api::dispatch(&app, "auto_review.set", params);

    // A provider the account has not connected cannot review.
    assert_eq!(set(json!({ "provider": "anthropic" })).await.unwrap_err(), "Anthropic is not connected");
    app.credentials.lock().unwrap().anthropic = Some(ApiKeyCredential { api_key: "sk-test".into(), base_url: None, connected_at: 1 });
    app.credentials.lock().unwrap().opencode = Some(ApiKeyCredential { api_key: "sk-test".into(), base_url: None, connected_at: 1 });

    // A provider alone takes its review model; a model of its own sticks.
    let result = set(json!({ "provider": "anthropic" })).await.unwrap();
    assert_eq!((result["auto_review"]["provider"].as_str(), result["auto_review"]["model"].as_str()), (Some("anthropic"), Some("claude-haiku-4-5")));
    set(json!({ "model": "claude-sonnet-5" })).await.unwrap();
    assert_eq!(app.auto_review().model.as_deref(), Some("claude-sonnet-5"));
    // Another provider starts on its own review model; the rest of the setting stays.
    let result = set(json!({ "provider": "opencode", "is_enabled": false })).await.unwrap();
    assert_eq!(result["auto_review"]["model"], "deepseek-v4.1-flash");
    let result = set(json!({ "provider": "opencode", "model": "jev-1.13-free" })).await.unwrap();
    assert_eq!((result["auto_review"]["model"].as_str(), result["auto_review"]["is_enabled"].as_bool()), (Some("jev-1.13-free"), Some(false)));
    // Back to the bot's own provider: neither is kept.
    let result = set(json!({ "provider": null })).await.unwrap();
    assert!(result["auto_review"].get("provider").is_none() && result["auto_review"].get("model").is_none(), "{result}");

    // The snapshot tells the apps which of the catalog's models decide.
    let snapshot = app.snapshot();
    let jev = snapshot["models"].as_array().unwrap().iter().find(|model| model["id"] == "jev-1.13").unwrap();
    assert_eq!((jev["provider"].as_str(), jev["decides"].as_bool()), (Some("opencode"), Some(true)));
}
