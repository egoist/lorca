//! Lorca's Device core for the phone. The app starts one `Core` with a folder to keep things
//! in and the facts about the phone, then speaks the same JSON API the desktop app speaks over
//! the local websocket: `request(method, params)` answers with `{ "result": … }` or
//! `{ "error": { "message": … } }`, and every event on the core's bus reaches the listener as
//! the `{ "event": …, "data": … }` frame the websocket would carry. No Runner here: a phone
//! takes part in chats and rooms and asks Runners for what only they hold.

uniffi::setup_scaffolding!();

/// Linked so the phone's bindings carry the Markdown parser's FFI beside the core's.
pub use lorca_markdown::parse_markdown;

use std::path::PathBuf;
use std::sync::Arc;

use lorca::app::App;
use lorca::config::Config;
use lorca::events::Event;

/// Where events go. Implemented by the app; called from the core's threads.
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, json: String);
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum CoreError {
    #[error("{0}")]
    Failed(String),
}

/// A push, opened: who replied, where, and the first words.
#[derive(uniffi::Record)]
pub struct PushNotice {
    pub title: String,
    pub subtitle: Option<String>,
    pub body: String,
    pub chat_id: String,
}

/// Opens the ciphertext a push carries (`c`, base64url) with the account key kept under
/// `home`. No running core needed: Android's messaging service calls this when a push wakes
/// the process. `None` when this phone is not paired or the push is not for this account.
#[uniffi::export]
pub fn push_open(home: String, sealed: String) -> Option<PushNotice> {
    let machine: lorca::keys::MachineFile = lorca::config::read_json(&Config { home: PathBuf::from(home), port: 0 }.machine_path())?;
    let notice = lorca::push::open(&machine.dek().ok()?, &lorca::keys::unb64(&sealed).ok()?).ok()?;
    Some(PushNotice { title: notice.title, subtitle: notice.subtitle, body: notice.body, chat_id: notice.chat_id })
}

/// One running core: the App, its runtime, and the sync loop.
#[derive(uniffi::Object)]
pub struct Core {
    app: Arc<App>,
    runtime: tokio::runtime::Runtime,
}

#[uniffi::export]
impl Core {
    /// Loads the core from `home` (the app's private folder) and starts syncing. `name`, `os`,
    /// `os_version`, and `model` describe this phone to the roster; `os` is `ios` or `android`,
    /// which is what makes the phone a Device and never a Runner.
    #[uniffi::constructor]
    pub fn start(home: String, name: String, os: String, os_version: String, model: String, listener: Arc<dyn EventListener>) -> Result<Arc<Self>, CoreError> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "lorca=info".into()))
            .with_target(false)
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .try_init();
        lorca::model::set_host_facts(name, os, os_version, model);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| CoreError::Failed(e.to_string()))?;
        let app = App::load(Config { home: PathBuf::from(home), port: 0 }).map_err(|e| CoreError::Failed(e.to_string()))?;
        let _guard = runtime.enter();
        lorca::runtime::prime_names(&app);
        runtime.spawn(lorca::sync::run(app.clone()));
        runtime.spawn(forward_events(app.clone(), listener));
        Ok(Arc::new(Core { app, runtime }))
    }

    /// One API call, the way the websocket takes it. Blocks the calling thread until the core
    /// answers, so call it off the UI thread.
    pub fn request(&self, method: String, params: String) -> String {
        let params: serde_json::Value = if params.trim().is_empty() { serde_json::Value::Null } else { serde_json::from_str(&params).unwrap_or(serde_json::Value::Null) };
        let app = self.app.clone();
        let response = self.runtime.block_on(async move {
            match lorca::api::dispatch(&app, &method, params).await {
                Ok(result) => serde_json::json!({ "result": result }),
                Err(message) => serde_json::json!({ "error": { "message": message } }),
            }
        });
        response.to_string()
    }

    /// The key pushes are sealed under, for the iOS notification service extension, which
    /// runs outside the app and reads it from the shared keychain. `None` until paired.
    pub fn push_key(&self) -> Option<Vec<u8>> {
        self.app.dek().map(|dek| lorca::keys::push_key(&dek).to_vec())
    }

    /// The app came to the foreground: sync now rather than after the backoff.
    pub fn wake(&self) {
        self.app.relay.forget_token();
        self.app.outbox_notify.notify_waiters();
    }
}

/// Every event on the bus, as its websocket frame. A listener that fell behind gets a fresh
/// snapshot, like a lagging websocket client.
async fn forward_events(app: Arc<App>, listener: Arc<dyn EventListener>) {
    let mut events = app.events.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => {
                if let Ok(text) = serde_json::to_string(&event) {
                    listener.on_event(text);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if let Ok(text) = serde_json::to_string(&Event::Snapshot(app.snapshot())) {
                    listener.on_event(text);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Collect(Mutex<Vec<String>>);
    impl EventListener for Collect {
        fn on_event(&self, json: String) {
            self.0.lock().unwrap().push(json);
        }
    }

    #[test]
    fn the_core_answers_the_api_and_forwards_events() {
        let home = std::env::temp_dir().join(format!("lorca-mobile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let listener = Arc::new(Collect(Mutex::new(Vec::new())));
        let core = Core::start(home.display().to_string(), "Phone".into(), "ios".into(), "iOS 26".into(), "iPhone17,1".into(), listener.clone()).unwrap();
        let hello: serde_json::Value = serde_json::from_str(&core.request("hello".into(), "{}".into())).unwrap();
        assert_eq!(hello["result"]["has_identity"], false);
        let bad: serde_json::Value = serde_json::from_str(&core.request("nope".into(), "".into())).unwrap();
        assert!(bad["error"]["message"].as_str().unwrap().contains("unknown method"));
        // Creating an identity emits identity.changed; the phone's facts name the Device.
        let created: serde_json::Value = serde_json::from_str(&core.request("identity.create".into(), r#"{"device_name":"Phone"}"#.into())).unwrap();
        assert!(created["result"]["phrase"].is_array());
        let snapshot: serde_json::Value = serde_json::from_str(&core.request("bootstrap".into(), "{}".into())).unwrap();
        let device = &snapshot["result"]["devices"][0];
        assert_eq!(device["os"], "ios");
        assert_eq!(device["name"], "Phone");
        std::thread::sleep(std::time::Duration::from_millis(200));
        let events = listener.0.lock().unwrap();
        assert!(events.iter().any(|e| e.contains("\"identity.changed\"")), "{events:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
