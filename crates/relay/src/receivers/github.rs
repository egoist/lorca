//! The Lorca GitHub App. A user installs it on the repositories bots may watch, or authorizes it
//! where it is installed already; either ends at `/webhooks/github/callback`, which binds to the account
//! whose `state` the link carried every installation the user's authorization reaches
//! (`GET /user/installations`), each with the repositories the user can access in it
//! (`GET /user/installations/{id}/repositories`). A routine subscribes a pull request
//! (`owner/repo#42`) only in a repository its account's binding lists, so nobody watches a
//! repository they can't see, even in an installation they share.
//! GitHub then sends the App's webhooks to `/webhooks/github`: their signature is checked with
//! the App's webhook secret, and each event about a subscribed pull request is turned into an
//! event in its own words and sealed to the routine's Runner. The App reads pull requests,
//! checks, statuses, issues, and actions; it writes nothing.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;

use super::Subscribe;
use crate::auth::{b64url_encode, Auth};
use crate::db::{now, ReceiverSub};
use crate::routes::{ApiError, ApiResult};
use crate::AppState;

/// The largest delivery the App takes: GitHub caps its own at 25 MB, and the events a watch
/// reads are far smaller.
pub const MAX_DELIVERY_BYTES: usize = 1024 * 1024;
/// How long an install link is good for.
const STATE_SECS: i64 = 3600;

pub struct GithubApp {
    pub app_id: String,
    pub slug: String,
    pub client_id: String,
    client_secret: String,
    webhook_secret: Vec<u8>,
    key: ring::signature::RsaKeyPair,
    /// `https://api.github.com`, and `https://github.com`; tests point them at a stub.
    pub api: String,
    pub web: String,
    /// The lorca.app page the install ends on.
    pub done_url: String,
    http: reqwest::Client,
}

impl GithubApp {
    /// The App, from its settings: the private key is the PEM GitHub gives (PKCS#1 or PKCS#8).
    #[allow(clippy::too_many_arguments)]
    pub fn new(app_id: &str, slug: &str, client_id: &str, client_secret: &str, webhook_secret: &str, private_key_pem: &str, done_url: &str) -> anyhow::Result<GithubApp> {
        let der = pem_der(private_key_pem)?;
        let key = ring::signature::RsaKeyPair::from_der(&der).or_else(|_| ring::signature::RsaKeyPair::from_pkcs8(&der)).map_err(|_| anyhow::anyhow!("The GitHub App's private key isn't an RSA key in PEM"))?;
        if webhook_secret.len() < 16 {
            anyhow::bail!("The GitHub App's webhook secret has at least 16 characters");
        }
        Ok(GithubApp {
            app_id: app_id.into(),
            slug: slug.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            webhook_secret: webhook_secret.as_bytes().to_vec(),
            key,
            api: std::env::var("LORCA_RELAY_GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".into()).trim_end_matches('/').into(),
            web: std::env::var("LORCA_RELAY_GITHUB_URL").unwrap_or_else(|_| "https://github.com".into()).trim_end_matches('/').into(),
            done_url: done_url.into(),
            http: reqwest::Client::builder().user_agent("lorca-relay").timeout(std::time::Duration::from_secs(20)).build()?,
        })
    }

    /// A link that installs the App and binds the installation to the identity that asked.
    pub async fn install_url(&self, state: &AppState, identity_pubkey: &str) -> ApiResult<String> {
        let nonce = super::random_id();
        state.db.receiver_state_put(&nonce, "github", identity_pubkey).await?;
        Ok(format!("{}/apps/{}/installations/new?state={nonce}", self.web, self.slug))
    }

    /// A link that has the user authorize the App where it is installed already, which binds what
    /// they can reach there to the identity that asked.
    pub async fn authorize_url(&self, state: &AppState, identity_pubkey: &str) -> ApiResult<String> {
        let nonce = super::random_id();
        state.db.receiver_state_put(&nonce, "github", identity_pubkey).await?;
        Ok(format!("{}/login/oauth/authorize?client_id={}&state={nonce}", self.web, urlencode(&self.client_id)))
    }

    /// The link for a pull request's repository: authorize where the App is installed, install
    /// where it isn't.
    pub async fn setup_url(&self, state: &AppState, identity_pubkey: &str, subject: Option<&str>) -> ApiResult<String> {
        match subject.and_then(pull_request) {
            Some((repo, _)) if self.installation_for(&repo).await?.is_some() => self.authorize_url(state, identity_pubkey).await,
            _ => self.install_url(state, identity_pubkey).await,
        }
    }

    /// The App's own token: an RS256 JWT for ten minutes less a minute of clock slack.
    fn jwt(&self) -> ApiResult<String> {
        let header = b64url_encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = b64url_encode(json!({ "iat": now() - 60, "exp": now() + 540, "iss": self.app_id }).to_string().as_bytes());
        let message = format!("{header}.{claims}");
        let mut signature = vec![0u8; self.key.public().modulus_len()];
        self.key
            .sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), message.as_bytes(), &mut signature)
            .map_err(|_| ApiError::internal("Signing the App's token failed"))?;
        Ok(format!("{message}.{}", b64url_encode(&signature)))
    }

    async fn call(&self, request: reqwest::RequestBuilder) -> ApiResult<Option<Value>> {
        let response = request.header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28").send().await.map_err(|error| {
            tracing::warn!(%error, "calling GitHub");
            ApiError::unavailable("GitHub can't be reached")
        })?;
        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if status.is_success() => Ok(Some(response.json().await.map_err(|_| ApiError::unavailable("GitHub answered something unreadable"))?)),
            status => {
                tracing::warn!(%status, "GitHub refused a request");
                Err(ApiError::unavailable("GitHub refused the request"))
            }
        }
    }

    /// The installation of the App on `repo`, and the account it is on.
    async fn installation_for(&self, repo: &str) -> ApiResult<Option<(String, String)>> {
        let found = self.call(self.http.get(format!("{}/repos/{repo}/installation", self.api)).bearer_auth(self.jwt()?)).await?;
        Ok(found.and_then(|found| Some((found["id"].as_u64()?.to_string(), found["account"]["login"].as_str().unwrap_or_default().to_string()))))
    }

    async fn installation_token(&self, installation: &str) -> ApiResult<String> {
        let found = self.call(self.http.post(format!("{}/app/installations/{installation}/access_tokens", self.api)).bearer_auth(self.jwt()?)).await?;
        found.and_then(|found| found["token"].as_str().map(str::to_string)).ok_or_else(|| ApiError::unavailable("GitHub gave the App no token"))
    }

    /// Subscribes a routine to a pull request: the App must be installed on the repository and
    /// bound to the routine's account, and the pull request open.
    pub async fn subscribe(&self, state: &AppState, auth: &Auth, body: &Subscribe) -> ApiResult<Value> {
        let Some((repo, number)) = pull_request(&body.subject) else {
            return Ok(json!({ "status": "refused", "message": "Give the pull request as owner/repo#42." }));
        };
        let setup = |url: String| json!({ "status": "needs_setup", "name": "GitHub", "setup_url": url });
        let Some((installation, _)) = self.installation_for(&repo).await? else { return Ok(setup(self.install_url(state, &auth.identity_pubkey).await?)) };
        // The repository must be one the user showed they can access: being able to reach the
        // installation is not enough, since an organization's covers repositories some members
        // can't see. A repository added since asks for the authorization again.
        let scope = state.db.receiver_scope("github", &installation, &auth.identity_pubkey).await?;
        let reachable = scope.as_deref().and_then(|scope| serde_json::from_str::<Vec<String>>(scope).ok()).is_some_and(|repos| repos.contains(&repo));
        if !reachable {
            return Ok(setup(self.authorize_url(state, &auth.identity_pubkey).await?));
        }
        let token = self.installation_token(&installation).await?;
        let Some(found) = self.call(self.http.get(format!("{}/repos/{repo}/pulls/{number}", self.api)).bearer_auth(token)).await? else {
            return Ok(json!({ "status": "refused", "message": format!("{repo} has no pull request #{number}.") }));
        };
        if found["state"].as_str() == Some("closed") {
            let how = if found["merged"].as_bool() == Some(true) { "merged" } else { "closed" };
            return Ok(json!({ "status": "refused", "message": format!("{repo}#{number} is already {how}, so there is nothing to watch.") }));
        }
        let kept = json!({ "given": body.subject, "head_sha": found["head"]["sha"] }).to_string();
        let sub = body.row("github", auth, format!("{repo}#{number}"), Some(installation), Some(kept));
        super::save(state, &sub).await?;
        Ok(json!({ "status": "subscribed", "id": sub.id, "name": "GitHub", "title": found["title"], "url": found["html_url"] }))
    }
}

/// `owner/repo#42` as the canonical `(owner/repo, 42)`, lowercase as GitHub's names compare.
pub fn pull_request(subject: &str) -> Option<(String, u64)> {
    let (repo, number) = subject.trim().split_once('#')?;
    let number: u64 = number.parse().ok().filter(|n| *n > 0)?;
    let (owner, name) = repo.split_once('/')?;
    let valid = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (valid(owner) && valid(name)).then(|| (repo.to_lowercase(), number))
}

fn pem_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    let body: String = pem.lines().filter(|line| !line.starts_with("-----")).map(str::trim).collect();
    Ok(base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body)?)
}

// MARK: - The install's end

#[derive(Debug, Deserialize)]
pub struct Callback {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    installation_id: Option<String>,
    #[serde(default)]
    setup_action: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// Where GitHub sends the user after the install and its authorization, or after an
/// authorization alone: binds to the account the state was made for every installation the
/// user's authorization reaches, with the repositories they can access in each, and ends on the
/// lorca.app page that says how it went.
pub async fn callback(State(state): State<AppState>, Query(query): Query<Callback>) -> Response {
    let Some(app) = &state.receivers.github else { return ApiError::not_found("This relay has no GitHub App").into_response() };
    let done = |status: &str, account: Option<&str>| {
        let mut url = format!("{}?status={status}", app.done_url);
        if let Some(account) = account {
            url.push_str(&format!("&account={}", urlencode(account)));
        }
        Redirect::to(&url).into_response()
    };
    let identity = match query.state.as_deref() {
        Some(nonce) => state.db.receiver_state_take(nonce, "github", now() - STATE_SECS).await.ok().flatten(),
        None => None,
    };
    let Some(identity) = identity else { return done("expired", None) };
    if query.setup_action.as_deref() == Some("request") {
        return done("requested", None);
    }
    let Some(code) = query.code.as_deref() else { return done("failed", None) };
    match bind(&state, app, &identity, code).await {
        // The installation GitHub named must be among the user's; an authorization alone names
        // none and binds whatever the user reaches.
        Ok(bound) => match query.installation_id.as_deref() {
            Some(installation) => match bound.iter().find(|(id, _)| id == installation) {
                Some((_, account)) => done("connected", Some(account)),
                None => done("not_yours", None),
            },
            None => match bound.first() {
                Some((_, account)) => done("connected", Some(account)),
                None => done("not_yours", None),
            },
        },
        Err(_) => done("failed", None),
    }
}

/// Binds every installation of the App the user's authorization reaches to the identity, each
/// with the repositories the user can access in it (lowercase `owner/repo`). The user's token
/// is used for this alone and kept nowhere. Returns the installations bound and their accounts.
async fn bind(state: &AppState, app: &GithubApp, identity: &str, code: &str) -> ApiResult<Vec<(String, String)>> {
    let exchanged = app
        .call(app.http.post(format!("{}/login/oauth/access_token", app.web)).json(&json!({ "client_id": app.client_id, "client_secret": app.client_secret, "code": code })))
        .await?
        .ok_or_else(|| ApiError::unavailable("GitHub has no sign-in for this code"))?;
    let token = exchanged["access_token"].as_str().ok_or_else(|| ApiError::unauthorized("GitHub turned the sign-in down"))?.to_string();
    let pages = |path: String, key: &'static str| {
        let token = token.clone();
        async move {
            let mut found = Vec::new();
            for page in 1..=10 {
                let listed = app.call(app.http.get(format!("{}{path}?per_page=100&page={page}", app.api)).bearer_auth(&token)).await?.unwrap_or(Value::Null);
                let items = listed[key].as_array().cloned().unwrap_or_default();
                let full = items.len() == 100;
                found.extend(items);
                if !full {
                    break;
                }
            }
            Ok::<_, ApiError>(found)
        }
    };
    let mut bound = Vec::new();
    for installation in pages("/user/installations".into(), "installations").await? {
        let Some(id) = installation["id"].as_u64().map(|id| id.to_string()) else { continue };
        let account = installation["account"]["login"].as_str().unwrap_or_default().to_string();
        let repos: Vec<String> = pages(format!("/user/installations/{id}/repositories"), "repositories")
            .await?
            .iter()
            .filter_map(|repo| repo["full_name"].as_str().map(str::to_lowercase))
            .collect();
        state.db.receiver_bind("github", &id, identity, &account, &serde_json::to_string(&repos).expect("serializable")).await?;
        bound.push((id, account));
    }
    Ok(bound)
}

fn urlencode(text: &str) -> String {
    text.bytes().map(|byte| if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect()
}

// MARK: - Deliveries

/// GitHub's signature over the exact body, checked in constant time.
pub fn signed(secret: &[u8], body: &[u8], header: &str) -> bool {
    let Some(hex) = header.strip_prefix("sha256=") else { return false };
    let Ok(given) = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok())).collect::<Option<Vec<u8>>>().ok_or(()) else { return false };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes any key");
    mac.update(body);
    mac.verify_slice(&given).is_ok()
}

/// A delivery from GitHub: `401` without the App's signature, `202` once every subscribed
/// pull request it concerns has its event sealed to its Runner.
pub async fn webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(app) = &state.receivers.github else { return ApiError::not_found("This relay has no GitHub App").into_response() };
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or_default().to_string();
    if !signed(&app.webhook_secret, &body, &header("x-hub-signature-256")) {
        return ApiError::unauthorized("Bad signature").into_response();
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else { return ApiError::bad_request("Not JSON").into_response() };
    let delivery = header("x-github-delivery");
    let delivery = if delivery.is_empty() { b64url_encode(&<Sha256 as sha2::Digest>::digest(&body)) } else { delivery };
    match route(&state, &header("x-github-event"), &delivery, &payload).await {
        Ok(sealed) => (StatusCode::ACCEPTED, Json(json!({ "sealed": sealed }))).into_response(),
        Err(error) => error.into_response(),
    }
}

/// Seals a delivery's event to the routines subscribed to what it concerns; an uninstall or a
/// repository taken off the App ends theirs. How many events were sealed.
pub async fn route(state: &AppState, event: &str, delivery: &str, payload: &Value) -> ApiResult<usize> {
    let installation = payload["installation"]["id"].as_u64().map(|id| id.to_string());
    let Some(seen) = normalize(event, payload) else {
        // The App taken away: whole, or from some repositories.
        let removed = match (event, payload["action"].as_str(), installation) {
            ("installation", Some("deleted"), Some(id)) => state.db.receiver_unbind("github", &id).await?,
            ("installation_repositories", Some("removed"), Some(id)) => {
                let mut removed = Vec::new();
                for repo in payload["repositories_removed"].as_array().into_iter().flatten().filter_map(|repo| repo["full_name"].as_str()) {
                    for sub in state.db.receiver_subs("github", &format!("{}#", repo.to_lowercase())).await? {
                        if sub.account.as_deref() == Some(id.as_str()) && state.db.receiver_unsubscribe(&sub.identity_pubkey, "github", &sub.id).await? {
                            removed.push(sub);
                        }
                    }
                }
                removed
            }
            _ => return Ok(0),
        };
        let mut sealed = 0;
        for sub in removed {
            let mut note = json!({ "kind": "unsubscribed", "summary": "The Lorca GitHub App was removed from this repository", "unsubscribed": true });
            note["subject"] = given(&sub);
            sealed += usize::from(super::deliver(state, &sub, &format!("{delivery}:{}", sub.id), &note).await?);
        }
        return Ok(sealed);
    };
    let mut sealed = 0;
    for sub in state.db.receiver_subs("github", &format!("{}#", seen.repo)).await? {
        let Some((_, number)) = pull_request(&sub.subject) else { continue };
        let kept: Value = sub.state.as_deref().and_then(|state| serde_json::from_str(state).ok()).unwrap_or(Value::Null);
        let head = kept["head_sha"].as_str().unwrap_or_default();
        let ours = seen.numbers.contains(&number) || (seen.numbers.is_empty() && !head.is_empty() && seen.sha.as_deref() == Some(head));
        if !ours {
            continue;
        }
        if let Some(new_head) = seen.head.as_deref().filter(|new_head| *new_head != head) {
            let mut kept = kept.clone();
            kept["head_sha"] = json!(new_head);
            state.db.receiver_set_state("github", &sub.id, &kept.to_string()).await?;
        }
        let mut event = seen.payload.clone();
        event["subject"] = given(&sub);
        if super::deliver(state, &sub, delivery, &event).await? {
            sealed += 1;
        }
        // Closed: its run is the routine's last, and the subscription goes.
        if seen.closes {
            state.db.receiver_unsubscribe(&sub.identity_pubkey, "github", &sub.id).await?;
        }
    }
    Ok(sealed)
}

/// The subject as the Runner gave it, which its filter compares.
fn given(sub: &ReceiverSub) -> Value {
    let kept: Value = sub.state.as_deref().and_then(|state| serde_json::from_str(state).ok()).unwrap_or(Value::Null);
    kept["given"].as_str().map(|given| json!(given)).unwrap_or_else(|| json!(sub.subject))
}

/// One GitHub event as a watch reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    /// `owner/repo`, lowercase.
    pub repo: String,
    /// The pull requests it names; none for a commit status, which names a commit.
    pub numbers: Vec<u64>,
    pub sha: Option<String>,
    /// A pull request's new head commit.
    pub head: Option<String>,
    pub closes: bool,
    /// The event for the Runner: `kind`, `summary`, `title`, `url`, `ends`, and trimmed `data`.
    pub payload: Value,
}

/// Conclusions that count as a failure of a check, a suite, or a workflow.
const FAILED: [&str; 4] = ["failure", "timed_out", "action_required", "startup_failure"];

/// What a GitHub event says about pull requests, or `None` when it starts no run: a label, an
/// assignment, a check that passed or is still running, a deleted comment.
pub fn normalize(event: &str, payload: &Value) -> Option<Seen> {
    let text = |value: &Value| value.as_str().filter(|text| !text.is_empty()).map(str::to_string);
    let clip = |value: &Value| value.as_str().map(|text| json!(text.chars().take(2000).collect::<String>())).unwrap_or(Value::Null);
    let action = payload["action"].as_str().unwrap_or_default();
    let numbers_of = |list: &Value| list.as_array().into_iter().flatten().filter_map(|pr| pr["number"].as_u64()).collect::<Vec<_>>();
    let pr = &payload["pull_request"];
    let mut actor = text(&payload["sender"]["login"]);
    let mut seen = Seen {
        repo: payload["repository"]["full_name"].as_str()?.to_lowercase(),
        numbers: pr["number"].as_u64().into_iter().collect(),
        sha: None,
        head: None,
        closes: false,
        payload: Value::Null,
    };
    let mut name = None;
    let mut data = json!({});
    let kind = match event {
        "pull_request" => {
            seen.head = text(&pr["head"]["sha"]);
            match action {
                "opened" => "opened",
                "synchronize" => "commits",
                "ready_for_review" => "ready",
                "converted_to_draft" => "draft",
                "edited" => "edited",
                "reopened" => "reopened",
                "closed" => {
                    seen.closes = true;
                    if pr["merged"].as_bool() == Some(true) { "merged" } else { "closed" }
                }
                _ => return None,
            }
        }
        "pull_request_review" if action == "submitted" => {
            actor = text(&payload["review"]["user"]["login"]).or(actor);
            data["review"] = json!({ "state": payload["review"]["state"], "body": clip(&payload["review"]["body"]), "url": payload["review"]["html_url"] });
            match payload["review"]["state"].as_str().unwrap_or_default().to_lowercase().as_str() {
                "approved" => "approved",
                "changes_requested" => "changes_requested",
                _ => "reviewed",
            }
        }
        "pull_request_review_comment" if action == "created" => {
            actor = text(&payload["comment"]["user"]["login"]).or(actor);
            data["comment"] = json!({ "body": clip(&payload["comment"]["body"]), "path": payload["comment"]["path"], "line": payload["comment"]["line"], "url": payload["comment"]["html_url"] });
            "review_comment"
        }
        "pull_request_review_thread" => match action {
            "resolved" => "thread_resolved",
            "unresolved" => "thread_unresolved",
            _ => return None,
        },
        "issue_comment" if action == "created" && payload["issue"]["pull_request"].is_object() => {
            seen.numbers = payload["issue"]["number"].as_u64().into_iter().collect();
            actor = text(&payload["comment"]["user"]["login"]).or(actor);
            data["comment"] = json!({ "body": clip(&payload["comment"]["body"]), "url": payload["comment"]["html_url"] });
            "comment"
        }
        "check_run" if action == "completed" && FAILED.contains(&payload["check_run"]["conclusion"].as_str().unwrap_or_default()) => {
            seen.numbers = numbers_of(&payload["check_run"]["pull_requests"]);
            seen.sha = text(&payload["check_run"]["head_sha"]);
            name = text(&payload["check_run"]["name"]);
            data["check"] = json!({ "name": payload["check_run"]["name"], "conclusion": payload["check_run"]["conclusion"], "url": payload["check_run"]["html_url"], "summary": clip(&payload["check_run"]["output"]["summary"]) });
            "check_failed"
        }
        "check_suite" if action == "completed" && FAILED.contains(&payload["check_suite"]["conclusion"].as_str().unwrap_or_default()) => {
            seen.numbers = numbers_of(&payload["check_suite"]["pull_requests"]);
            seen.sha = text(&payload["check_suite"]["head_sha"]);
            name = text(&payload["check_suite"]["app"]["name"]);
            data["check_suite"] = json!({ "app": payload["check_suite"]["app"]["name"], "conclusion": payload["check_suite"]["conclusion"] });
            "checks_failed"
        }
        "status" if matches!(payload["state"].as_str(), Some("failure" | "error")) => {
            seen.sha = text(&payload["sha"]);
            name = text(&payload["context"]);
            data["status"] = json!({ "context": payload["context"], "state": payload["state"], "description": clip(&payload["description"]), "url": payload["target_url"] });
            "status_failed"
        }
        "workflow_run" if action == "completed" && FAILED.contains(&payload["workflow_run"]["conclusion"].as_str().unwrap_or_default()) => {
            seen.numbers = numbers_of(&payload["workflow_run"]["pull_requests"]);
            seen.sha = text(&payload["workflow_run"]["head_sha"]);
            name = text(&payload["workflow_run"]["name"]);
            data["workflow"] = json!({ "name": payload["workflow_run"]["name"], "conclusion": payload["workflow_run"]["conclusion"], "url": payload["workflow_run"]["html_url"] });
            "workflow_failed"
        }
        _ => return None,
    };
    data["event"] = json!(event);
    data["action"] = json!(action);
    if pr.is_object() {
        data["pull_request"] = json!({ "number": pr["number"], "state": pr["state"], "draft": pr["draft"], "merged": pr["merged"], "head": pr["head"]["sha"], "user": pr["user"]["login"] });
    }
    let mut out = json!({ "kind": kind, "summary": summary(kind, actor.as_deref(), name.as_deref()), "actor": actor, "data": data });
    if let Some(title) = text(&pr["title"]).or_else(|| text(&payload["issue"]["title"])) {
        out["title"] = json!(title);
    }
    if let Some(url) = text(&pr["html_url"]).or_else(|| text(&payload["issue"]["html_url"])) {
        out["url"] = json!(url);
    }
    if seen.closes {
        out["ends"] = json!(true);
    }
    seen.payload = out;
    Some(seen)
}

/// What happened, in a line: "Changes requested by kim", "Check failed: test (ubuntu)".
pub fn summary(kind: &str, actor: Option<&str>, name: Option<&str>) -> String {
    let by = |what: &str| actor.map(|actor| format!("{what} by {actor}")).unwrap_or_else(|| what.to_string());
    let named = |what: &str| name.map(|name| format!("{what}: {name}")).unwrap_or_else(|| what.to_string());
    match kind {
        "opened" => by("Opened"),
        "commits" => "New commits pushed".into(),
        "ready" => "Ready for review".into(),
        "draft" => "Turned back into a draft".into(),
        "edited" => "Edited".into(),
        "reopened" => "Reopened".into(),
        "merged" => "Merged".into(),
        "closed" => "Closed without merging".into(),
        "approved" => by("Approved"),
        "changes_requested" => by("Changes requested"),
        "reviewed" => by("Reviewed"),
        "review_comment" => by("Review comment"),
        "thread_resolved" => "Review thread resolved".into(),
        "thread_unresolved" => "Review thread reopened".into(),
        "comment" => by("Comment"),
        "check_failed" => named("Check failed"),
        "checks_failed" => named("Checks failed"),
        "status_failed" => named("Status failed"),
        "workflow_failed" => named("Workflow failed"),
        other => other.to_string(),
    }
}
