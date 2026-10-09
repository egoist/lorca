//! Shared MCP call admission for every bot using one installed account on this Runner.
//! Rate windows/cooldowns and configuration survive restart as account ciphertext.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::config::now_secs;

const PURPOSE: &str = "runner_connector_limits_v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CallLimits {
    pub max_calls: u32,
    pub window_secs: u64,
    pub max_concurrency: u32,
}

impl Default for CallLimits {
    fn default() -> Self {
        Self {
            max_calls: 60,
            window_secs: 60,
            max_concurrency: 4,
        }
    }
}

impl CallLimits {
    fn validate(&self) -> Result<(), String> {
        if self.window_secs == 0 || self.window_secs > 86_400 {
            return Err("The rate window must be between 1 second and 24 hours.".into());
        }
        if self.max_concurrency > 256 || self.max_calls > 10_000 {
            return Err("The connector limit is too large.".into());
        }
        Ok(())
    }
}

/// The service an installed account belongs to: a named account's marketplace service, or a
/// single-account plugin's own id. Its account labels never pick a bucket.
fn service_of(app: &App, plugin_id: &str) -> String {
    app.plugins.lock().unwrap().get(plugin_id).map(|plugin| plugin.service_id().to_string()).unwrap_or_else(|| plugin_id.to_string())
}

fn account_key(plugin_id: &str) -> String {
    format!("account:{plugin_id}")
}
fn service_key(service: &str) -> String {
    format!("service:{service}")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Bucket {
    starts: VecDeque<f64>,
    cooldown_until: f64,
    #[serde(skip)]
    active: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    limits: BTreeMap<String, CallLimits>,
    buckets: BTreeMap<String, Bucket>,
}

#[derive(Default)]
pub struct ConnectorLimits {
    state: Mutex<Option<State>>,
    changed: Notify,
}

impl ConnectorLimits {
    pub fn clear(&self) {
        *self.state.lock().unwrap() = None;
        self.changed.notify_waiters();
    }

    fn load<'a>(&self, app: &App, held: &'a mut Option<State>) -> Result<&'a mut State, String> {
        if held.is_none() {
            let dek = app.dek().ok_or("Create or pair an identity first.")?;
            *held = Some(
                match app
                    .store
                    .runner_limits(PURPOSE)
                    .map_err(|e| e.to_string())?
                {
                    Some(bytes) => crate::crypto::decrypt_json(&dek, PURPOSE, &bytes)
                        .map_err(|e| format!("Cannot read connector limits: {e}"))?,
                    None => State::default(),
                },
            );
        }
        Ok(held.as_mut().unwrap())
    }

    fn save(&self, app: &App, state: &State) -> Result<(), String> {
        let dek = app.dek().ok_or("The account is no longer available.")?;
        let bytes = crate::crypto::encrypt_json(&dek, PURPOSE, state).map_err(|e| e.to_string())?;
        app.store
            .set_runner_limits(PURPOSE, &bytes)
            .map_err(|e| format!("Cannot persist connector admission: {e}"))
    }

    /// Transport handshakes/reconnection also honor guidance, even before a logical tool
    /// call obtains its rate permit. Dropping the HTTP future cancels this wait.
    pub async fn wait_cooldown(app: &Arc<App>, plugin_id: &str) -> Result<(), String> {
        if !app.has_identity() {
            return Ok(());
        }
        loop {
            let notified = app.connector_limits.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let remaining = {
                let mut held = app.connector_limits.state.lock().unwrap();
                let state = app.connector_limits.load(app, &mut held)?;
                state
                    .buckets
                    .get(&account_key(plugin_id))
                    .map(|bucket| bucket.cooldown_until - now_secs())
                    .unwrap_or(0.0)
            };
            if remaining <= 0.0 {
                return Ok(());
            }
            tokio::select! {
                _ = &mut notified => {},
                _ = tokio::time::sleep(Duration::from_secs_f64(remaining.min(1.0))) => {},
            }
        }
    }

    /// All servers/namespaces share the account bucket and the service bucket. Named account
    /// IDs from #71 retain separate account capacities within the service's shared capacity.
    pub async fn acquire(
        app: &Arc<App>,
        plugin_id: &str,
        cancel: &CancellationToken,
    ) -> Result<CallPermit, String> {
        let keys = [account_key(plugin_id), service_key(&service_of(app, plugin_id))];
        loop {
            if cancel.is_cancelled() {
                return Err("Stopped before the connector call.".into());
            }
            let notified = app.connector_limits.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let wait = {
                let mut held = app.connector_limits.state.lock().unwrap();
                let state = app.connector_limits.load(app, &mut held)?;
                let now = now_secs();
                let mut until = now;
                let mut available = true;
                for key in &keys {
                    let limits = state.limits.get(key).cloned().unwrap_or_default();
                    if limits.max_calls == 0 || limits.max_concurrency == 0 {
                        return Err("Connector calls are paused by this account's or service's shared limit. Increase the limit in its settings.".into());
                    }
                    let bucket = state.buckets.entry(key.clone()).or_default();
                    while bucket
                        .starts
                        .front()
                        .is_some_and(|t| *t + limits.window_secs as f64 <= now)
                    {
                        bucket.starts.pop_front();
                    }
                    until = until.max(bucket.cooldown_until);
                    if bucket.starts.len() >= limits.max_calls as usize {
                        until = until.max(bucket.starts[0] + limits.window_secs as f64);
                    }
                    available &= bucket.active < limits.max_concurrency;
                }
                if until <= now && available {
                    for key in &keys {
                        let bucket = state.buckets.get_mut(key).unwrap();
                        bucket.starts.push_back(now);
                        bucket.active += 1;
                    }
                    if let Err(error) = app.connector_limits.save(app, state) {
                        for key in &keys {
                            let bucket = state.buckets.get_mut(key).unwrap();
                            bucket.starts.pop_back();
                            bucket.active -= 1;
                        }
                        return Err(error);
                    }
                    return Ok(CallPermit {
                        app: app.clone(),
                        keys: keys.into(),
                    });
                }
                Duration::from_secs_f64((until - now).max(0.05).min(1.0))
            };
            tokio::select! {
                _ = cancel.cancelled() => return Err("Stopped while waiting for the account's connector limit.".into()),
                _ = &mut notified => {},
                _ = tokio::time::sleep(wait) => {},
            }
        }
    }

    /// Holds the account's calls for what the service asked, at most a day.
    pub fn cooldown(&self, app: &App, plugin_id: &str, seconds: f64) {
        if !seconds.is_finite() || seconds < 0.0 {
            return;
        }
        let seconds = seconds.min(86_400.0);
        let mut held = self.state.lock().unwrap();
        let result = self.load(app, &mut held).and_then(|state| {
            let bucket = state.buckets.entry(account_key(plugin_id)).or_default();
            bucket.cooldown_until = bucket.cooldown_until.max(now_secs() + seconds);
            self.save(app, state)
        });
        if let Err(error) = result {
            tracing::error!(%error, "keeping service retry guidance");
        }
        self.changed.notify_waiters();
    }

    /// Structured MCP retry guidance is data from the service. It delays future calls and
    /// never authorizes replaying the failed operation.
    pub fn observe_result(&self, app: &App, plugin_id: &str, result: &Value) {
        let data = result
            .get("data")
            .or_else(|| result.get("structuredContent"))
            .unwrap_or(result);
        let number = |key: &str| {
            data.get(key)
                .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        };
        if let Some(seconds) = number("retry_after_ms")
            .or_else(|| number("retryAfterMs"))
            .map(|v| v / 1000.0)
            .or_else(|| number("retry_after"))
            .or_else(|| number("retryAfter"))
        {
            self.cooldown(app, plugin_id, seconds);
        }
    }
}

/// Dropping a completed, failed, or cancelled call always returns concurrency capacity.
pub struct CallPermit {
    app: Arc<App>,
    keys: Vec<String>,
}

impl Drop for CallPermit {
    fn drop(&mut self) {
        if let Some(state) = self.app.connector_limits.state.lock().unwrap().as_mut() {
            for key in &self.keys {
                if let Some(bucket) = state.buckets.get_mut(key) {
                    bucket.active = bucket.active.saturating_sub(1);
                }
            }
        }
        self.app.connector_limits.changed.notify_waiters();
    }
}

pub async fn dispatch(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    let runner = params["runner_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| app.this_device_id())
        .ok_or("missing runner_id")?;
    if app.this_device_id().as_deref() != Some(runner.as_str()) {
        return crate::requests::ask(app, &runner, method, params.clone()).await;
    }
    serve(app, method, params)
}

pub fn serve(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    let plugin_id = params["plugin_id"].as_str().ok_or("missing plugin_id")?;
    let service = app.plugins.lock().unwrap().get(plugin_id).map(|plugin| plugin.service_id().to_string()).ok_or("Unknown installed account on this Runner.")?;
    let mut held = app.connector_limits.state.lock().unwrap();
    let state = app.connector_limits.load(app, &mut held)?;
    let key = match params["scope"].as_str().unwrap_or("account") {
        "account" => account_key(plugin_id),
        "service" => service_key(&service),
        _ => return Err("scope must be account or service".into()),
    };
    if method == "connector_limits.set" {
        let limits: CallLimits =
            serde_json::from_value(params["limits"].clone()).map_err(|e| e.to_string())?;
        limits.validate()?;
        let mut changed = state.clone();
        changed.limits.insert(key.clone(), limits);
        // A new limit is the user's call; it ends a wait the service asked for.
        if let Some(bucket) = changed.buckets.get_mut(&account_key(plugin_id)) {
            bucket.cooldown_until = 0.0;
        }
        app.connector_limits.save(app, &changed)?;
        *state = changed;
        app.connector_limits.changed.notify_waiters();
    } else if method != "connector_limits.get" {
        return Err(format!("Unknown connector limits method {method}"));
    }
    let limits = state.limits.get(&key).cloned().unwrap_or_default();
    let bucket = state.buckets.get(&key).cloned().unwrap_or_default();
    Ok(
        json!({ "plugin_id": plugin_id, "service_id": service, "limits": limits, "retry_at": (bucket.cooldown_until > now_secs()).then_some(bucket.cooldown_until) }),
    )
}

mod http;
pub use http::LimitedHttpClient;

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) struct Scratch(pub Arc<App>, std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    pub(super) fn scratch() -> Scratch {
        let home = std::env::temp_dir().join(format!("lorca-limit-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config {
            home: home.clone(),
            port: 0,
        })
        .unwrap();
        crate::identity::create(&app, Some("Limiter Runner".into())).unwrap();
        Scratch(app, home)
    }
    fn set(app: &Arc<App>, plugin_id: &str, limits: CallLimits) {
        let mut held = app.connector_limits.state.lock().unwrap();
        let state = app.connector_limits.load(app, &mut held).unwrap();
        state.limits.insert(account_key(plugin_id), limits);
        app.connector_limits.save(app, state).unwrap();
    }

    #[tokio::test]
    async fn bots_share_account_capacity_cancelled_waits_do_not_leak_and_other_accounts_progress() {
        let scratch = scratch();
        let app = &scratch.0;
        set(
            app,
            "work",
            CallLimits {
                max_concurrency: 1,
                ..CallLimits::default()
            },
        );
        set(
            app,
            "personal",
            CallLimits {
                max_concurrency: 1,
                ..CallLimits::default()
            },
        );
        let cancel = CancellationToken::new();
        let first = ConnectorLimits::acquire(app, "work", &cancel)
            .await
            .unwrap();
        let other = ConnectorLimits::acquire(app, "personal", &cancel)
            .await
            .unwrap();
        let queued_cancel = CancellationToken::new();
        let cloned = app.clone();
        let token = queued_cancel.clone();
        let queued =
            tokio::spawn(async move { ConnectorLimits::acquire(&cloned, "work", &token).await });
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            !queued.is_finished(),
            "another bot on the account waits for its shared permit"
        );
        queued_cancel.cancel();
        assert!(queued.await.unwrap().is_err());
        drop(first);
        drop(other);
        let later = tokio::time::timeout(
            Duration::from_secs(1),
            ConnectorLimits::acquire(app, "work", &cancel),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            app.connector_limits
                .state
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .buckets[&account_key("work")]
                .active,
            1
        );
        drop(later);
        assert_eq!(
            app.connector_limits
                .state
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .buckets[&account_key("work")]
                .active,
            0
        );
    }

    #[tokio::test]
    async fn rate_windows_and_service_cooldowns_survive_restart_without_plaintext() {
        let scratch = scratch();
        let app = &scratch.0;
        set(
            app,
            "account-secret-name",
            CallLimits {
                max_calls: 1,
                window_secs: 60,
                max_concurrency: 4,
            },
        );
        drop(
            ConnectorLimits::acquire(app, "account-secret-name", &CancellationToken::new())
                .await
                .unwrap(),
        );
        app.connector_limits
            .cooldown(app, "account-secret-name", 120.0);
        let bytes = app.store.runner_limits(PURPOSE).unwrap().unwrap();
        assert!(!bytes.windows(19).any(|w| w == b"account-secret-name"));
        app.connector_limits.clear();
        let cancel = CancellationToken::new();
        let pending = ConnectorLimits::acquire(app, "account-secret-name", &cancel);
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(Duration::from_millis(60), &mut pending)
                .await
                .is_err()
        );
        cancel.cancel();
        assert!(pending.await.is_err());
        let held = app.connector_limits.state.lock().unwrap();
        let state = held.as_ref().unwrap();
        assert_eq!(
            state.buckets[&account_key("account-secret-name")]
                .starts
                .len(),
            1
        );
        assert!(
            state.buckets[&account_key("account-secret-name")].cooldown_until > now_secs() + 100.0
        );
        assert_eq!(state.buckets[&account_key("account-secret-name")].active, 0);
    }

    #[tokio::test]
    async fn zero_capacity_fails_actionably_and_mcp_guidance_never_shortens_a_cooldown() {
        let scratch = scratch();
        let app = &scratch.0;
        set(
            app,
            "paused",
            CallLimits {
                max_calls: 0,
                ..CallLimits::default()
            },
        );
        assert!(
            ConnectorLimits::acquire(app, "paused", &CancellationToken::new())
                .await
                .err()
                .unwrap()
                .contains("Increase the limit")
        );
        app.connector_limits
            .observe_result(app, "work", &json!({"data":{"retryAfterMs":2000}}));
        let until = app
            .connector_limits
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .buckets[&account_key("work")]
            .cooldown_until;
        app.connector_limits.observe_result(
            app,
            "work",
            &json!({"structuredContent":{"retry_after":0.01}}),
        );
        assert_eq!(
            app.connector_limits
                .state
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .buckets[&account_key("work")]
                .cooldown_until,
            until
        );
    }

    #[tokio::test]
    async fn named_accounts_share_the_services_concurrency_cap_without_a_fallback_account() {
        let scratch = scratch();
        let app = &scratch.0;
        let manifest = crate::plugins::Manifest::parse(&json!({ "id": "gmail", "name": "Gmail", "named_accounts": true,
            "servers": { "api": { "type": "http", "url": "https://gmail.test/mcp", "auth": { "type": "oauth" } } } }))
        .unwrap();
        let work = crate::plugins::accounts::install(app, manifest.clone(), "marketplace", Some("Work")).unwrap().id;
        let personal = crate::plugins::accounts::install(app, manifest, "marketplace", Some("Personal")).unwrap().id;
        assert_ne!(account_key(&work), account_key(&personal));
        assert_eq!(service_of(app, &work), "gmail");
        assert_eq!(service_of(app, &personal), "gmail");
        {
            let mut held = app.connector_limits.state.lock().unwrap();
            let state = app.connector_limits.load(app, &mut held).unwrap();
            state.limits.insert(
                service_key("gmail"),
                CallLimits {
                    max_concurrency: 1,
                    ..CallLimits::default()
                },
            );
        }
        let first = ConnectorLimits::acquire(app, &work, &CancellationToken::new())
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let second = ConnectorLimits::acquire(app, &personal, &cancel);
        tokio::pin!(second);
        assert!(tokio::time::timeout(Duration::from_millis(60), &mut second)
            .await
            .is_err());
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .unwrap()
            .unwrap();
        drop(second);
    }
}
