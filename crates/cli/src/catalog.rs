//! Keeps the model catalog current without a new release. The catalog in use is the latest of
//! the one this build carries, the one cached in `catalog.json`, and the one served at
//! [`DEFAULT_URL`] (`LORCA_MODELS_URL`; `LORCA_MODELS_FETCH=0` turns fetching off). `lorca serve`
//! checks the server when it starts, as the phone's core does when its app first bootstraps;
//! after that an app's `bootstrap` checks, and `models.reload` checks even within the hour. A
//! check within the hour of the last one is skipped, and nothing checks on a timer. A turn on a
//! built-in provider's model the catalog lacks checks first, at most every five minutes, since
//! another Device may have offered it from a newer catalog.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::config::{self, Config};
use crate::served;

pub const DEFAULT_URL: &str = "https://lorca.app/models/v1.json";
/// How often a turn on a model the catalog lacks may check.
const UNKNOWN_MODEL_EVERY: Duration = Duration::from_secs(5 * 60);

/// This Device's checks for a newer catalog.
#[derive(Default)]
pub struct Updates {
    /// Where newer catalogs come from. Unset until [`enable`], so an App that never enables it,
    /// a test or a one-shot command, reads only the bundled and cached catalogs.
    url: OnceLock<String>,
    /// One check at a time.
    checking: tokio::sync::Mutex<()>,
    /// When a turn on a model the catalog lacks last checked.
    unknown_checked: Mutex<Option<Instant>>,
}

/// `catalog.json` in Lorca's folder: the last catalog fetched, as served, and what the next
/// check sends back.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    /// Unix seconds of the last check, answered or not.
    #[serde(default)]
    checked_at: i64,
    #[serde(default)]
    catalog: Value,
}

/// Lets this Device check for newer catalogs, at `LORCA_MODELS_URL` or lorca.app, unless
/// `LORCA_MODELS_FETCH=0`.
pub fn enable(app: &App) {
    if std::env::var("LORCA_MODELS_FETCH").is_ok_and(|value| value.trim() == "0") {
        return;
    }
    let url = std::env::var("LORCA_MODELS_URL").ok().map(|url| url.trim().to_string()).filter(|url| !url.is_empty());
    let _ = app.catalog.url.set(url.unwrap_or_else(|| DEFAULT_URL.into()));
}

/// Installs the cached catalog when it is later than the one this build carries.
pub fn load_cached(config: &Config) {
    let Some(cache) = config::read_json::<Cache>(&config.catalog_path()) else { return };
    if cache.catalog.is_null() {
        return;
    }
    match lorca_models::parse(&cache.catalog.to_string()) {
        Ok(catalog) => {
            lorca_models::install(catalog);
        }
        Err(error) => tracing::warn!(%error, "reading the cached model catalog"),
    }
}

/// Checks for a newer catalog in the background, unless updates are off.
pub fn check_in_background(app: &Arc<App>) {
    if app.catalog.url.get().is_none() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = check(&app, false).await {
            tracing::warn!(%error, "checking for a newer model catalog");
        }
    });
}

/// Before a turn: a built-in provider's model the catalog lacks may be in a newer catalog,
/// picked on a Device that has it. Checks now, at most every five minutes.
pub async fn check_for_model(app: &Arc<App>, provider: &str, model: Option<&str>) {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else { return };
    if app.catalog.url.get().is_none() || lorca_models::default_model(provider).is_none() || lorca_models::find(provider, model).is_some() {
        return;
    }
    {
        let mut last = app.catalog.unknown_checked.lock().unwrap();
        if last.is_some_and(|at| at.elapsed() < UNKNOWN_MODEL_EVERY) {
            return;
        }
        *last = Some(Instant::now());
    }
    if let Err(error) = check(app, true).await {
        tracing::warn!(%error, provider, model, "checking for a newer model catalog");
    }
}

/// Asks the server for its catalog, unless the last check was within the hour and this one is
/// not forced. A later catalog than the one in use is installed and the apps hear its models in
/// `roster.changed`. True when one was installed.
pub async fn check(app: &Arc<App>, force: bool) -> Result<bool, String> {
    let Some(url) = app.catalog.url.get() else { return Err("Model catalog updates are off".into()) };
    let _one_at_a_time = app.catalog.checking.lock().await;
    let path = app.config.catalog_path();
    let mut cache: Cache = config::read_json(&path).unwrap_or_default();
    let now = config::now_unix();
    if !force && (0..served::FRESH_SECS).contains(&(now - cache.checked_at)) {
        return Ok(false);
    }
    let fetched = fetch(app, url, &cache).await;
    cache.checked_at = now;
    let installed = match fetched {
        Ok(Some((etag, text, catalog))) => {
            let installed = lorca_models::install(catalog);
            // Kept even when it is not newer than the one in use, so the next check can ask
            // whether it changed.
            cache.catalog = serde_json::from_str(&text).unwrap_or_default();
            cache.etag = etag;
            Ok(installed)
        }
        Ok(None) => Ok(false),
        Err(error) => Err(error),
    };
    if let Err(error) = config::write_json_private(&path, &cache) {
        tracing::warn!(%error, "saving the model catalog");
    }
    if installed == Ok(true) {
        app.emit(app.roster_summary());
    }
    installed
}

/// The catalog at `url` with its ETag, or `None` when it has not changed since the cached one.
async fn fetch(app: &App, url: &str, cache: &Cache) -> Result<Option<(Option<String>, String, lorca_models::Catalog)>, String> {
    // The validator goes only with the catalog it names, so a 304 always leaves one to keep.
    let etag = cache.etag.as_deref().filter(|_| !cache.catalog.is_null());
    let Some((etag, text)) = served::fetch(app, url, etag).await? else { return Ok(None) };
    let catalog = lorca_models::parse(&text)?;
    Ok(Some((etag, text, catalog)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Event;

    struct Scratch(Arc<App>, std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> Scratch {
        let home = std::env::temp_dir().join(format!("lorca-catalog-{}", uuid::Uuid::new_v4()));
        Scratch(App::load(Config { home: home.clone(), port: 0 }).unwrap(), home)
    }

    #[tokio::test]
    async fn a_later_catalog_is_fetched_once_cached_and_revalidated() {
        // The bundled models under a later time, so other tests' lookups answer the same.
        let mut later: Value = serde_json::from_str(lorca_models::BUNDLED).unwrap();
        later["updated"] = Value::from("2999-01-01T00:00:00Z");
        let (url, seen) = served::test_server("/models/v1.json", later.to_string());
        let scratch = scratch_app();
        let app = &scratch.0;
        assert_eq!(check(app, true).await, Err("Model catalog updates are off".into()));
        app.catalog.url.set(url).unwrap();
        let mut events = app.events.subscribe();

        assert_eq!(check(app, false).await, Ok(true));
        assert_eq!(lorca_models::updated(), "2999-01-01T00:00:00Z");
        let cache: Cache = config::read_json(&app.config.catalog_path()).unwrap();
        assert_eq!((cache.etag.as_deref(), cache.catalog["updated"].as_str()), (Some("\"v1\""), Some("2999-01-01T00:00:00Z")));
        let heard = std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event {
            Event::RosterChanged { models, .. } => Some(models),
            _ => None,
        });
        assert_eq!(heard.map(|models| models.len()), Some(lorca_models::models().len()));

        // Within the hour nothing is asked; forced, the server is asked whether it changed.
        assert_eq!(check(app, false).await, Ok(false));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(check(app, true).await, Ok(false));
        assert!(seen.lock().unwrap()[1].contains("\r\nif-none-match: \"v1\"\r\n"));

        // A turn checks for a built-in provider's model the catalog lacks, at most every five
        // minutes, and never for a known model or a custom provider's.
        check_for_model(app, "anthropic", Some("claude-opus-5")).await;
        check_for_model(app, "custom:lab", Some("qwen3:8b")).await;
        assert_eq!(seen.lock().unwrap().len(), 2);
        check_for_model(app, "anthropic", Some("claude-opus-9")).await;
        assert_eq!(seen.lock().unwrap().len(), 3);
        check_for_model(app, "anthropic", Some("claude-opus-9")).await;
        assert_eq!(seen.lock().unwrap().len(), 3);
    }
}
