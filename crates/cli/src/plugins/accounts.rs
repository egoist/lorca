//! Named connections are installed plugin instances, with the same Runner-local secrets and
//! encrypted management requests. The service id identifies a marketplace entry; the instance
//! id selects one account, independent of its label and authorization lifetime.

use std::sync::Arc;

use super::{announce, install_instance, Manifest, Store};
use crate::{app::App, model::PluginStatus};

fn name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Give the account a name.".into());
    }
    if value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err("Use a name of up to 80 characters, on one line.".into());
    }
    Ok(value.to_string())
}

/// `service_name` is the service as the user reads it, such as Gmail.
pub(crate) fn check_unique(store: &Store, service: &str, service_name: &str, account: &str, except: Option<&str>) -> Result<(), String> {
    if store.instances(service).any(|plugin| {
        Some(plugin.manifest.id.as_str()) != except && plugin.account_name.as_ref().is_some_and(|label| label.to_lowercase() == account.to_lowercase())
    }) {
        return Err(format!("There is already a {service_name} account named {account}."));
    }
    Ok(())
}

pub fn install(app: &Arc<App>, mut manifest: Manifest, source: &str, account_name: Option<&str>) -> Result<PluginStatus, String> {
    manifest.check()?;
    if !manifest.named_accounts {
        return Err("This plugin does not support named accounts.".into());
    }
    let service = manifest.id.clone();
    // A blank name becomes the next free `Account N`.
    let account = match account_name.filter(|value| !value.trim().is_empty()) {
        Some(value) => name(value)?,
        None => {
            let store = app.plugins.lock().unwrap();
            (1..).map(|n| format!("Account {n}")).find(|label| check_unique(&store, &service, &manifest.name, label, None).is_ok()).expect("a free account name")
        }
    };
    check_unique(&app.plugins.lock().unwrap(), &service, &manifest.name, &account, None)?;
    manifest.id = format!("{service}-{}", uuid::Uuid::new_v4().simple());
    install_instance(app, manifest, source, Some(service), Some(account))
}

/// Renaming does not move secrets, pooled connections, codemode tool names, or tool grants.
pub fn rename(app: &Arc<App>, id: &str, account_name: &str) -> Result<PluginStatus, String> {
    let account = name(account_name)?;
    let status = {
        let mut store = app.plugins.lock().unwrap();
        let plugin = store.get(id).ok_or("Unknown plugin")?;
        let (service, service_name) = (plugin.service_id.clone().ok_or("This is not a named account.")?, plugin.manifest.name.clone());
        check_unique(&store, &service, &service_name, &account, Some(id))?;
        let plugin = store.installed.iter_mut().find(|plugin| plugin.manifest.id == id).ok_or("Unknown plugin")?;
        plugin.account_name = Some(account);
        store.save(&app.config).map_err(|error| error.to_string())?;
        store.status(id).ok_or("Unknown plugin")?
    };
    announce(app);
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{self, Store};
    use serde_json::json;

    #[test]
    fn account_ids_and_secrets_survive_rename_restart_and_marketplace_refresh() {
        let home = std::env::temp_dir().join(format!("lorca-accounts-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        let manifest = Manifest::parse(&json!({ "id": "gmail", "name": "Gmail", "named_accounts": true,
            "servers": { "api": { "type": "http", "url": "https://gmail.test/mcp", "auth": { "type": "oauth" } } } }))
        .unwrap();
        let work = install(&app, manifest.clone(), "marketplace", Some("Work")).unwrap();
        let personal = install(&app, manifest.clone(), "marketplace", Some("Personal")).unwrap();
        assert_ne!(work.id, personal.id);
        assert!(work.id.starts_with("gmail-") && work.id.len() == 38);
        assert_eq!(work.service_id.as_deref(), Some("gmail"));
        assert_eq!(install(&app, manifest.clone(), "marketplace", Some("work")).unwrap_err(), "There is already a Gmail account named work.");
        let unnamed = install(&app, manifest.clone(), "marketplace", Some(" ")).unwrap();
        assert_eq!(unnamed.account_name.as_deref(), Some("Account 1"), "a blank name is the next free one");
        plugins::uninstall(&app, &unnamed.id).unwrap();
        assert!(rename(&app, &work.id, " ").is_err());
        assert!(rename(&app, &work.id, "Personal").is_err());
        plugins::set_oauth(&app, &work.id, "api", Some(json!({ "client_id": "c", "tokens": { "access_token": "work-secret" } }))).unwrap();
        plugins::set_oauth(&app, &personal.id, "api", Some(json!({ "client_id": "c", "tokens": { "access_token": "personal-secret" } }))).unwrap();
        let renamed = rename(&app, &work.id, "Office").unwrap();
        assert_eq!(renamed.id, work.id);
        assert_eq!(renamed.name, "Gmail · Office");
        let mut updated = manifest;
        updated.version = "2".into();
        assert_eq!(plugins::refresh_installed(&app, &[updated]), vec![work.id.clone(), personal.id.clone()]);
        let reloaded = Store::load(&app.config);
        assert_eq!(reloaded.instances("gmail").count(), 2);
        assert_eq!(reloaded.get(&work.id).unwrap().manifest.version, "2");
        assert_eq!(reloaded.status(&work.id).unwrap().account_name.as_deref(), Some("Office"));
        assert_eq!(reloaded.sign_in_secret(&work.id, "oauth", "api").unwrap()["tokens"]["access_token"], "work-secret");
        let public = serde_json::to_string(&reloaded.statuses()).unwrap() + &plugins::detail(&app, &work.id).unwrap().to_string();
        assert!(!public.contains("work-secret") && !public.contains("personal-secret"));
        #[cfg(feature = "runner")]
        let generation = app.mcp.generation(&work.id);
        plugins::sign_out(&app, &work.id, None).unwrap();
        #[cfg(feature = "runner")]
        assert!(plugins::set_oauth_at_generation(&app, &work.id, "api", json!({ "tokens": { "access_token": "late-refresh" } }), generation).is_err(), "a late refresh cannot undo sign-out");
        assert_eq!(app.plugins.lock().unwrap().status(&work.id).unwrap().state, "needs_auth");
        assert_eq!(app.plugins.lock().unwrap().status(&personal.id).unwrap().state, "ready");
        plugins::uninstall(&app, &work.id).unwrap();
        assert!(app.plugins.lock().unwrap().get(&personal.id).is_some());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn legacy_installs_have_one_service_and_default_labels_do_not_collide() {
        let old: super::super::Installed =
            serde_json::from_value(json!({ "manifest": { "id": "legacy", "name": "Legacy", "servers": {} }, "source": "inline", "installed_at": 0 })).unwrap();
        assert_eq!(old.service_id(), "legacy");
        assert_eq!(old.display_name(), "Legacy");
        assert!(old.account_name.is_none());
        assert!(name("x\nname").is_err());
        assert!(name(&"a".repeat(81)).is_err());
    }
}
