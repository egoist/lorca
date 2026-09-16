//! What travels with every model call besides the messages: headers, a timeout, a session id
//! for cache affinity, provider metadata, and hooks that see the key, the payload, and the
//! response. After pi's stream options and its `before_payload` / `after_response` hooks.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

/// The status and headers of a model call's response, as the hooks see them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseInfo {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
}

/// Hooks the adapters call around one request. Every method has a default that does nothing.
#[async_trait]
pub trait RequestHooks: Send + Sync {
    /// A key to use for this call instead of the adapter's own, for tokens that expire.
    async fn api_key(&self) -> Option<String> {
        None
    }

    /// The request body, right before it is sent. Change it in place.
    fn before_payload(&self, _payload: &mut Value) {}

    /// The response, once its status and headers are in and before its body streams.
    fn after_response(&self, _response: &ResponseInfo) {}
}

/// Per-call request settings. `Default` is no headers, no timeout, no hooks.
#[derive(Clone, Default)]
pub struct RequestOptions {
    /// Extra headers, merged over the adapter's own; a caller's value wins.
    pub headers: BTreeMap<String, String>,
    /// The whole request, connect to last byte. `None` is the adapter's default (no limit).
    pub timeout: Option<Duration>,
    /// A conversation id for providers that route by session to reuse their prompt cache:
    /// sent as `x-session-affinity`, and as `prompt_cache_key` where the API has it.
    pub session_id: Option<String>,
    /// Provider metadata; the Anthropic adapter forwards `user_id`.
    pub metadata: BTreeMap<String, Value>,
    pub hooks: Option<Arc<dyn RequestHooks>>,
}

impl std::fmt::Debug for RequestOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestOptions")
            .field("headers", &self.headers)
            .field("timeout", &self.timeout)
            .field("session_id", &self.session_id)
            .field("metadata", &self.metadata)
            .field("hooks", &self.hooks.as_ref().map(|_| "…"))
            .finish()
    }
}

impl RequestOptions {
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_session_id(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_hooks(mut self, hooks: Arc<dyn RequestHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Applies a patch: given fields replace, a header or metadata value of `None` deletes.
    pub fn apply(&mut self, patch: RequestOptionsPatch) {
        for (name, value) in patch.headers {
            match value {
                Some(value) => {
                    self.headers.insert(name, value);
                }
                None => {
                    self.headers.remove(&name);
                }
            }
        }
        if let Some(timeout) = patch.timeout {
            self.timeout = timeout;
        }
        if let Some(session_id) = patch.session_id {
            self.session_id = session_id;
        }
        for (name, value) in patch.metadata {
            match value {
                Some(value) => {
                    self.metadata.insert(name, value);
                }
                None => {
                    self.metadata.remove(&name);
                }
            }
        }
    }

    /// Gives the hooks the body to change before it is sent.
    pub fn before_payload(&self, body: &mut Value) {
        if let Some(hooks) = &self.hooks {
            hooks.before_payload(body);
        }
    }

    /// Applies the options to a request: the affinity header, the extra headers, the timeout.
    pub fn apply_to(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(session_id) = &self.session_id {
            request = request.header("x-session-affinity", session_id);
        }
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(timeout) = self.timeout {
            request = request.timeout(timeout);
        }
        request
    }

    /// The key for this call: the hooks' override, else the adapter's own.
    pub async fn api_key(&self, own: &str) -> String {
        match &self.hooks {
            Some(hooks) => hooks.api_key().await.unwrap_or_else(|| own.to_string()),
            None => own.to_string(),
        }
    }

    /// Reports the response to the hooks.
    pub fn report(&self, response: &reqwest::Response) {
        let Some(hooks) = &self.hooks else { return };
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.as_str().to_string(), v.to_string())))
            .collect();
        hooks.after_response(&ResponseInfo { status: response.status().as_u16(), headers });
    }
}

/// A change to [`RequestOptions`]: `None` fields are left alone, a header or metadata entry of
/// `None` is removed.
#[derive(Debug, Clone, Default)]
pub struct RequestOptionsPatch {
    pub headers: BTreeMap<String, Option<String>>,
    pub timeout: Option<Option<Duration>>,
    pub session_id: Option<Option<String>>,
    pub metadata: BTreeMap<String, Option<Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Spy(Mutex<Vec<String>>);

    #[async_trait]
    impl RequestHooks for Spy {
        async fn api_key(&self) -> Option<String> {
            Some("fresh".into())
        }
        fn before_payload(&self, payload: &mut Value) {
            payload["seen"] = Value::Bool(true);
            self.0.lock().unwrap().push("payload".into());
        }
        fn after_response(&self, response: &ResponseInfo) {
            self.0.lock().unwrap().push(format!("response {}", response.status));
        }
    }

    #[tokio::test]
    async fn options_prepare_the_request_and_override_the_key() {
        let spy = Arc::new(Spy(Mutex::new(vec![])));
        let options = RequestOptions::default().with_header("x-test", "1").with_session_id("s1").with_timeout(Duration::from_secs(9)).with_hooks(spy.clone());
        let client = reqwest::Client::new();
        let mut body = serde_json::json!({ "model": "m" });
        options.before_payload(&mut body);
        let request = options.apply_to(client.post("http://127.0.0.1:1/")).build().unwrap();
        assert_eq!(body["seen"], true);
        assert_eq!(request.headers().get("x-test").unwrap(), "1");
        assert_eq!(request.headers().get("x-session-affinity").unwrap(), "s1");
        assert_eq!(request.timeout(), Some(&Duration::from_secs(9)));
        assert_eq!(options.api_key("own").await, "fresh");
        assert_eq!(RequestOptions::default().api_key("own").await, "own");
    }

    #[test]
    fn a_patch_replaces_and_removes() {
        let mut options = RequestOptions::default().with_header("a", "1").with_header("b", "2");
        let mut patch = RequestOptionsPatch::default();
        patch.headers.insert("a".into(), None);
        patch.headers.insert("c".into(), Some("3".into()));
        patch.session_id = Some(Some("s".into()));
        options.apply(patch);
        assert_eq!(options.headers, BTreeMap::from([("b".to_string(), "2".to_string()), ("c".to_string(), "3".to_string())]));
        assert_eq!(options.session_id.as_deref(), Some("s"));
    }
}
