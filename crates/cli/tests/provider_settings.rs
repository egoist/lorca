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

    // A provider reviews; the rest of the setting stays.
    let result = set(json!({ "provider": "anthropic", "is_enabled": false })).await.unwrap();
    assert_eq!((result["auto_review"]["provider"].as_str(), result["auto_review"]["is_enabled"].as_bool()), (Some("anthropic"), Some(false)));
    // Each provider's review model is picked on its own; the ones left out keep theirs.
    set(json!({ "models": { "anthropic": "claude-sonnet-5" } })).await.unwrap();
    let result = set(json!({ "models": { "opencode": "jev-1.13-free" } })).await.unwrap();
    assert_eq!(result["auto_review"]["models"], json!({ "anthropic": "claude-sonnet-5", "opencode": "jev-1.13-free" }));
    // Null puts a provider back on its default, and a provider back on the bot's.
    let result = set(json!({ "provider": null, "models": { "anthropic": null } })).await.unwrap();
    assert!(result["auto_review"].get("provider").is_none(), "{result}");
    assert_eq!(result["auto_review"]["models"], json!({ "opencode": "jev-1.13-free" }));

    // Each provider's status names the model it reviews with by default.
    let statuses = app.credentials.lock().unwrap().statuses();
    let default = |kind: &str| statuses.iter().find(|status| status.kind == kind).and_then(|status| status.review_model.clone());
    assert_eq!((default("anthropic").as_deref(), default("opencode").as_deref()), (Some("claude-haiku-4-5"), Some("deepseek-v4.1-flash")));

    // The snapshot tells the apps which of the catalog's models decide.
    let snapshot = app.snapshot();
    let jev = snapshot["models"].as_array().unwrap().iter().find(|model| model["id"] == "jev-1.13").unwrap();
    assert_eq!((jev["provider"].as_str(), jev["decides"].as_bool()), (Some("opencode"), Some(true)));
}

#[tokio::test]
async fn a_provider_kept_on_one_runner_is_offered_for_its_bots_alone() {
    let home = Home(std::env::temp_dir().join(format!("lorca-provider-sync-{}", uuid::Uuid::new_v4())));
    let app = App::load(Config { home: home.0.clone(), port: 0 }).unwrap();
    app.credentials.lock().unwrap().deepseek = Some(ApiKeyCredential { api_key: "sk-test-kept".into(), base_url: None, connected_at: 1 });

    // Only a connected provider can stop syncing.
    assert!(api::dispatch(&app, "providers.set_sync", json!({ "kind": "anthropic", "sync": false })).await.is_err());
    let result = api::dispatch(&app, "providers.set_sync", json!({ "kind": "deepseek", "sync": false })).await.unwrap();
    let deepseek = result["providers"].as_array().unwrap().iter().find(|p| p["kind"] == "deepseek").unwrap().clone();
    assert_eq!(deepseek["kept"], true);
    // The set this Device sends has no key; the file it keeps still does.
    let shared = serde_json::to_string(&app.credentials.lock().unwrap().shared()).unwrap();
    assert!(!shared.contains("sk-test-kept") && !shared.contains("\"kept\""));
    let saved = std::fs::read_to_string(app.config.credentials_path()).unwrap();
    assert!(saved.contains("sk-test-kept") && saved.contains("\"kept\""));

    // Another Runner's kept provider reaches this Device's statuses through its machine blob.
    let mut studio: lorca::model::Device = serde_json::from_value(json!({
        "id": "studio", "name": "Studio", "model": "", "os": "linux", "os_version": "", "box_pubkey": "",
        "providers": [{ "kind": "anthropic", "is_connected": true, "detail": "", "kept": true }],
    }))
    .unwrap();
    studio.updated_at = 1;
    app.state.lock().unwrap().devices.push(studio);
    let statuses = app.provider_statuses();
    let anthropic = statuses.iter().find(|status| status.kind == "anthropic").unwrap();
    assert_eq!((anthropic.is_connected, anthropic.runners.as_slice(), anthropic.detail.as_str()), (false, ["studio".to_string()].as_slice(), "Kept on Studio"));
    let error = lorca::providers::provider_for(&app, "anthropic", None, None).err().unwrap();
    assert_eq!(error, "Anthropic is kept on Studio. Connect it on this Device too, or sync it from there.");

    // Syncing again sends the key with the next set.
    api::dispatch(&app, "providers.set_sync", json!({ "kind": "deepseek", "sync": true })).await.unwrap();
    assert!(serde_json::to_string(&app.credentials.lock().unwrap().shared()).unwrap().contains("sk-test-kept"));
}

#[tokio::test]
async fn claude_code_is_this_runners_own_and_never_in_the_shared_set() {
    let home = Home(std::env::temp_dir().join(format!("lorca-claude-code-{}", uuid::Uuid::new_v4())));
    let app = App::load(Config { home: home.0.clone(), port: 0 }).unwrap();
    let status = |app: &std::sync::Arc<App>| app.provider_statuses().into_iter().find(|status| status.kind == "claude-code").unwrap();

    lorca::claude_code::set_status(Some(lorca::claude_code::Status { program: None, ..Default::default() }));
    let missing = status(&app);
    assert!(!missing.is_connected && missing.detail.contains("not installed"));
    // It is listed after Anthropic, as the apps list the built-ins.
    let kinds: Vec<String> = app.provider_statuses().into_iter().map(|status| status.kind).collect();
    assert_eq!(&kinds[..3], ["deepseek", "anthropic", "claude-code"]);
    let error = lorca::providers::provider_for(&app, "claude-code", None, None).err().unwrap();
    assert!(error.starts_with("Claude Code is not installed on this Runner"), "{error}");

    lorca::claude_code::set_status(Some(lorca::claude_code::Status { program: Some("/usr/local/bin/claude".into()), signed_in: true, plan: Some("max".into()), email: Some("me@example.com".into()) }));
    let signed_in = status(&app);
    assert_eq!((signed_in.is_connected, signed_in.kept, signed_in.detail.as_str()), (true, true, "Claude Max · me@example.com"));
    assert!(app.connected_kinds().contains(&"claude-code".to_string()));
    let provider = lorca::providers::provider_for(&app, "claude-code", None, None).unwrap();
    assert_eq!((provider.provider_id(), provider.model_id()), ("claude-code", "claude-opus-5-5"));
    // The account's set never carries it, and Disconnect points at Claude Code's own sign-out.
    assert!(!serde_json::to_string(&app.credentials.lock().unwrap().shared()).unwrap().contains("claude-code"));
    assert!(api::dispatch(&app, "providers.disconnect", json!({ "kind": "claude-code" })).await.unwrap_err().contains("claude auth logout"));
    lorca::claude_code::set_status(None);
}
