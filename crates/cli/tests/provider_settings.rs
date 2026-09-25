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
    let kinds = ["deepseek", "anthropic", "opencode", "opencode-go", "cerebras"];

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
        credentials.cerebras = Some(ApiKeyCredential { api_key: "sk-test-cerebras-secret".into(), base_url: None, connected_at: 1 });
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
async fn cerebras_resolves_from_its_api_key() {
    let home = Home(std::env::temp_dir().join(format!("lorca-provider-settings-{}", uuid::Uuid::new_v4())));
    let app = App::load(Config { home: home.0.clone(), port: 0 }).unwrap();
    assert_eq!(lorca::providers::provider_for(&app, "cerebras", None, None).err().as_deref(), Some("Cerebras is not connected"));

    app.credentials.lock().unwrap().cerebras = Some(ApiKeyCredential { api_key: "csk-test".into(), base_url: None, connected_at: 1 });
    let provider = lorca::providers::provider_for(&app, "cerebras", None, None).unwrap();
    assert_eq!((provider.provider_id(), provider.model_id()), ("cerebras", "qwen-3.8-27b"));
    assert_eq!(provider.model_info().map(|m| m.context_window), Some(131_072));
    assert!(provider.supports_images());

    let qwen = lorca::providers::provider_for(&app, "cerebras", Some("qwen-3.8-27b"), None).unwrap();
    assert_eq!(qwen.model_id(), "qwen-3.8-27b");
    assert!(qwen.supports_images());
    assert!(lorca::providers::supports_vision("cerebras", Some("qwen-3.8-27b")));
    assert!(lorca::providers::supports_vision("cerebras", None));
}
