//! Email for an account: one address per identity on the relay's mail domain, which every bot of
//! the account shares. The mail Worker (`mail/`) asks where an address's mail goes, seals a copy
//! to each machine that takes it, and hands the copies back as `event` envelopes, so mail rests
//! here only as ciphertext a Runner opens. Mail out goes through Email Sending's REST API with
//! the account's address as the sender, within the day's allowance. The relay keeps nothing about
//! bots: the `+tag` a bot writes from is passed with each send and kept nowhere.

use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{b64url_decode, Auth};
use crate::db::{self, now, MailAddress};
use crate::routes::{ApiError, ApiResult};
use crate::AppState;

/// A Device's first days on the relay send at most `new_daily_sends`.
const NEW_ACCOUNT_SECS: i64 = 7 * 86_400;
/// How long an address that kept bouncing stays suspended.
const SUSPENSION_SECS: i64 = 7 * 86_400;
/// Names nobody may pick: the mailboxes mail standards and abuse desks expect, and the
/// project's own.
const RESERVED: &[&str] = &[
    "abuse", "admin", "administrator", "billing", "bounce", "bounces", "contact", "dmarc", "email", "help", "hostmaster", "info",
    "legal", "lorca", "mail", "mailer-daemon", "marketing", "no-reply", "noc", "noreply", "postmaster", "privacy", "root", "sales",
    "security", "staff", "support", "system", "team", "webmaster", "www",
];

pub struct MailConfig {
    /// `bots.lorca.app`: addresses are `<name>@<domain>` and `<name>+<tag>@<domain>`.
    pub domain: String,
    /// What the mail Worker sends as its bearer.
    pub worker_token: String,
    /// Email Sending, when the operator gave its account and token. Without it mail comes in
    /// and bots cannot send.
    pub sending: Option<Sending>,
    pub daily_sends: i64,
    pub new_daily_sends: i64,
    pub max_recipients: usize,
    /// Bounces in a day that suspend an address.
    pub bounce_limit: i64,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/mail/address", get(address).post(claim).delete(release))
        .route("/v1/mail/runner", put(runner))
        .route("/v1/mail/send", post(send))
        .route("/v1/mail/route/{name}", get(route))
        .route("/v1/mail/deliveries/{id}", put(deliver))
}

fn config(state: &AppState) -> ApiResult<&MailConfig> {
    state.mail.as_deref().ok_or_else(|| ApiError::not_found("This relay has no email"))
}

// MARK: - Names

/// The name as the relay keeps it, lowercase, or why it cannot be one.
pub fn check_name(name: &str) -> ApiResult<String> {
    let name = name.trim().to_ascii_lowercase();
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-';
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    if !(3..=32).contains(&name.len()) || !name.chars().all(allowed) || !edge(name.chars().next()) || !edge(name.chars().last()) || name.contains("..") {
        return Err(ApiError::coded(StatusCode::BAD_REQUEST, "Use 3 to 32 letters, digits, dots, and hyphens", "invalid"));
    }
    if RESERVED.contains(&name.as_str()) {
        return Err(ApiError::coded(StatusCode::BAD_REQUEST, "That name is reserved", "reserved"));
    }
    Ok(name)
}

/// Eight letters and digits that read unambiguously, starting with a letter: `k7f3m9q2`.
fn random_name() -> String {
    const LETTERS: &[u8] = b"abcdefghjkmnpqrstuvwxyz";
    const ANY: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    let mut name = String::with_capacity(8);
    name.push(LETTERS[rng.gen_range(0..LETTERS.len())] as char);
    for _ in 0..7 {
        name.push(ANY[rng.gen_range(0..ANY.len())] as char);
    }
    name
}

/// `scout` in `name+scout@domain`: lowercase letters, digits, and hyphens.
fn check_tag(tag: &str) -> ApiResult<()> {
    if tag.is_empty() || tag.len() > 40 || !tag.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(ApiError::bad_request("A sender tag is 1 to 40 lowercase letters, digits, and hyphens"));
    }
    Ok(())
}

// MARK: - A Device's address

fn view(config: &MailConfig, address: Option<MailAddress>, runner: bool) -> Value {
    json!({
        "domain": config.domain,
        "address": address.map(|address| json!({
            "name": address.name,
            "suspended_until": address.suspended_until.filter(|until| *until > now()),
        })),
        "runner": runner,
    })
}

async fn current(state: &AppState, auth: &Auth) -> ApiResult<Json<Value>> {
    let config = config(state)?;
    let address = state.db.mail_address(&auth.identity_pubkey).await?;
    let runner = state.db.is_mail_runner(&auth.machine_pubkey).await?;
    Ok(Json(view(config, address, runner)))
}

/// The account's address, and whether this machine takes its mail.
async fn address(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    current(&state, &auth).await
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    #[serde(default)]
    name: Option<String>,
}

/// Gives the account an address, or another one: the name asked for when it is free and
/// allowed, a random one without a name. The address it held is given up.
async fn claim(State(state): State<AppState>, auth: Auth, Json(body): Json<Claim>) -> ApiResult<Json<Value>> {
    config(&state)?;
    match body.name.as_deref() {
        Some(name) => {
            let name = check_name(name)?;
            state.db.claim_mail_address(&auth.identity_pubkey, &name).await?;
        }
        None => {
            let mut attempts = 0;
            loop {
                match state.db.claim_mail_address(&auth.identity_pubkey, &random_name()).await {
                    Ok(_) => break,
                    Err(error) if error.status() == StatusCode::CONFLICT && attempts < 8 => attempts += 1,
                    Err(error) => return Err(error),
                }
            }
        }
    }
    state.db.publish(db::Event::Mail { identity: auth.identity_pubkey.clone() }).await;
    current(&state, &auth).await
}

/// Gives the address up. Mail to it bounces from now on, and the name stays this account's.
async fn release(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    config(&state)?;
    if state.db.release_mail_address(&auth.identity_pubkey).await? {
        state.db.publish(db::Event::Mail { identity: auth.identity_pubkey.clone() }).await;
    }
    current(&state, &auth).await
}

/// This machine takes the account's mail: a Runner says so once it sees the account has an
/// address. Unpairing the machine ends it.
async fn runner(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    config(&state)?;
    state.db.set_mail_runner(&auth.identity_pubkey, &auth.machine_pubkey).await?;
    current(&state, &auth).await
}

// MARK: - The mail Worker

/// The mail Worker, by the token it shares with the relay.
pub struct MailWorker;

impl FromRequestParts<AppState> for MailWorker {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let config = config(state)?;
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError::unauthorized("Missing bearer token"))?;
        // Compared in constant time, digest against digest, so neither length nor bytes leak.
        let (given, expected) = (<sha2::Sha256 as sha2::Digest>::digest(token.as_bytes()), <sha2::Sha256 as sha2::Digest>::digest(config.worker_token.as_bytes()));
        if given.iter().zip(expected.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
            return Err(ApiError::unauthorized("Not the mail Worker"));
        }
        Ok(MailWorker)
    }
}

/// Where mail to `name` goes: the machines that take its mail and the box keys to seal it to.
async fn route(State(state): State<AppState>, _worker: MailWorker, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    let name = name.to_ascii_lowercase();
    let route = state.db.mail_route(&name).await?.ok_or_else(|| ApiError::not_found("No such address"))?;
    if route.address.is_suspended() {
        return Ok(Json(json!({ "state": "suspended", "machines": [] })));
    }
    let machines: Vec<Value> = route.machines.iter().map(|(machine, key)| json!({ "machine_pubkey": machine, "box_pubkey": key })).collect();
    Ok(Json(json!({ "state": "active", "machines": machines })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Delivery {
    name: String,
    machine_pubkey: String,
    ciphertext: String,
}

/// One sealed copy of a message, for one machine: stored as an `event` envelope only that
/// machine reads, under the identity's quota, and deleted once it is consumed or after seven
/// days, as other envelopes are.
async fn deliver(State(state): State<AppState>, _worker: MailWorker, Path(id): Path<String>, Json(body): Json<Delivery>) -> ApiResult<Json<Value>> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(ApiError::bad_request("Delivery id must be 1–64 characters of [A-Za-z0-9._-]"));
    }
    let route = state.db.mail_route(&body.name.to_ascii_lowercase()).await?.ok_or_else(|| ApiError::not_found("No such address"))?;
    if route.address.is_suspended() {
        return Err(ApiError::forbidden("Address suspended"));
    }
    if !route.machines.iter().any(|(machine, _)| *machine == body.machine_pubkey) {
        return Err(ApiError::conflict("That machine takes no mail for this address"));
    }
    let ciphertext = db::blocking(move || b64url_decode(&body.ciphertext)).await?;
    if ciphertext.is_empty() || ciphertext.len() > crate::routes::MAX_BLOB_BYTES {
        return Err(ApiError::too_large("Message too large"));
    }
    let blob = db::NewBlob {
        identity_pubkey: route.identity_pubkey.clone(),
        id: id.clone(),
        kind: "event".into(),
        recipient_machine_pubkey: Some(body.machine_pubkey.clone()),
        slot: None,
        group: None,
        payload: db::Payload::Inline(ciphertext),
    };
    let inserted = state.db.insert_blob(blob, state.quota_bytes).await?;
    if !inserted.existing {
        state.db.publish(db::Event::Blobs { identity: route.identity_pubkey, recipient: Some(body.machine_pubkey) }).await;
    }
    Ok(Json(json!({ "id": id, "seq": inserted.seq })))
}

// MARK: - Sending

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SendRequest {
    /// The bot's tag: the message goes out from `name+tag@domain`, so replies come back to it.
    #[serde(default)]
    tag: Option<String>,
    /// The sender's display name, the bot's.
    #[serde(default)]
    from_name: Option<String>,
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    subject: String,
    text: String,
    #[serde(default)]
    html: Option<String>,
    #[serde(default)]
    in_reply_to: Option<String>,
    #[serde(default)]
    references: Option<String>,
    #[serde(default)]
    attachments: Vec<Attachment>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    filename: String,
    #[serde(rename = "type")]
    content_type: String,
    /// Standard base64.
    content: String,
}

fn one_line(text: &str, max: usize, what: &str) -> ApiResult<()> {
    if text.len() > max || text.contains(['\r', '\n']) {
        return Err(ApiError::bad_request(&format!("{what} must be one line of at most {max} bytes")));
    }
    Ok(())
}

/// `someone@example.com`: one `@`, a dotted domain, nothing that would split a header.
fn check_recipient(address: &str, domain: &str) -> ApiResult<String> {
    let address = address.trim();
    let valid = address.len() <= 254
        && !address.contains(|c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"' | '(' | ')'))
        && address.split_once('@').is_some_and(|(local, host)| !local.is_empty() && host.contains('.') && !host.contains('@') && !host.starts_with('.') && !host.ends_with('.'));
    if !valid {
        return Err(ApiError::bad_request(&format!("Not an email address: {address}")));
    }
    // Bots write to each other in Lorca. Mail between two addresses here would loop.
    if address.rsplit_once('@').is_some_and(|(_, host)| host.eq_ignore_ascii_case(domain)) {
        return Err(ApiError::bad_request("Bots don't email addresses on this domain"));
    }
    Ok(address.to_string())
}

/// Sends a message from the account's address through Email Sending, within the day's
/// allowance. Its permanent bounces and suppressed recipients count against the address.
async fn send(State(state): State<AppState>, auth: Auth, Json(body): Json<SendRequest>) -> ApiResult<Json<Value>> {
    let config = config(&state)?;
    let sending = config.sending.as_ref().ok_or_else(|| ApiError::unavailable("This relay can't send email"))?;
    let address = state.db.mail_address(&auth.identity_pubkey).await?.ok_or_else(|| ApiError::coded(StatusCode::FORBIDDEN, "This account has no email address", "no_address"))?;
    if address.is_suspended() {
        return Err(ApiError::coded(StatusCode::FORBIDDEN, "The address is suspended: mail from it kept bouncing", "suspended"));
    }
    if let Some(tag) = &body.tag {
        check_tag(tag)?;
    }
    let to: Vec<String> = body.to.iter().map(|a| check_recipient(a, &config.domain)).collect::<ApiResult<_>>()?;
    let cc: Vec<String> = body.cc.iter().map(|a| check_recipient(a, &config.domain)).collect::<ApiResult<_>>()?;
    if to.is_empty() || to.len() + cc.len() > config.max_recipients {
        return Err(ApiError::bad_request(&format!("A message goes to 1 to {} recipients", config.max_recipients)));
    }
    one_line(&body.subject, 998, "The subject")?;
    if let Some(name) = &body.from_name {
        one_line(name, 64, "The sender's name")?;
    }
    for header in [&body.in_reply_to, &body.references].into_iter().flatten() {
        one_line(header, 2048, "A threading header")?;
    }
    if body.text.trim().is_empty() {
        return Err(ApiError::bad_request("A message needs text"));
    }
    if body.attachments.len() > 10 {
        return Err(ApiError::bad_request("A message carries at most 10 attachments"));
    }
    for attachment in &body.attachments {
        one_line(&attachment.filename, 255, "An attachment's name")?;
        one_line(&attachment.content_type, 255, "An attachment's type")?;
    }

    let day = now() / 86_400;
    let usage = state.db.mail_usage(&auth.identity_pubkey, day).await?;
    let allowance = if now() - usage.identity_created_at < NEW_ACCOUNT_SECS { config.new_daily_sends } else { config.daily_sends };
    if usage.sent >= allowance {
        let tomorrow = (day + 1) * 86_400 - now();
        return Err(ApiError::coded(StatusCode::TOO_MANY_REQUESTS, "The account sent as much email today as it may", "daily_limit").retry_after(tomorrow.max(1) as u64));
    }

    let from = match &body.tag {
        Some(tag) => format!("{}+{tag}@{}", address.name, config.domain),
        None => format!("{}@{}", address.name, config.domain),
    };
    let mut headers = serde_json::Map::new();
    if let Some(id) = &body.in_reply_to {
        headers.insert("In-Reply-To".into(), json!(id));
    }
    if let Some(references) = &body.references {
        headers.insert("References".into(), json!(references));
    }
    let mut message = json!({
        "from": match &body.from_name { Some(name) => json!({ "address": from, "name": name }), None => json!(from) },
        "to": to,
        "subject": body.subject,
        "text": body.text,
    });
    if !cc.is_empty() {
        message["cc"] = json!(cc);
    }
    if let Some(html) = &body.html {
        message["html"] = json!(html);
    }
    if !headers.is_empty() {
        message["headers"] = Value::Object(headers);
    }
    if !body.attachments.is_empty() {
        message["attachments"] = json!(body.attachments.iter().map(|a| json!({ "filename": a.filename, "type": a.content_type, "content": a.content, "disposition": "attachment" })).collect::<Vec<_>>());
    }
    let sent = sending.send(&message).await?;
    let bounces = (sent.permanent_bounces.len() + sent.suppressed.len()) as i64;
    let suspended = state.db.record_mail_send(&auth.identity_pubkey, day, bounces, config.bounce_limit, now() + SUSPENSION_SECS).await?;
    if suspended {
        tracing::warn!(identity = %auth.identity_pubkey, "suspended an email address that kept bouncing");
        state.db.publish(db::Event::Mail { identity: auth.identity_pubkey.clone() }).await;
    }
    Ok(Json(json!({
        "from": from,
        "message_id": sent.message_id,
        "delivered": sent.delivered,
        "queued": sent.queued,
        "bounced": sent.permanent_bounces,
        "suppressed": sent.suppressed,
    })))
}

/// Email Sending's REST API for one Cloudflare account. `LORCA_RELAY_EMAIL_API_URL` points it
/// at a test server.
pub struct Sending {
    url: String,
    account_id: String,
    token: String,
    http: reqwest::Client,
}

#[derive(Debug, Default)]
pub struct Sent {
    pub message_id: Option<String>,
    pub delivered: Vec<String>,
    pub queued: Vec<String>,
    pub permanent_bounces: Vec<String>,
    pub suppressed: Vec<String>,
}

impl Sending {
    pub fn new(account_id: String, token: String, url: Option<String>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        let url = url.unwrap_or_else(|| "https://api.cloudflare.com/client/v4".into()).trim_end_matches('/').to_string();
        Ok(Sending { url, account_id, token, http })
    }

    async fn send(&self, message: &Value) -> ApiResult<Sent> {
        let url = format!("{}/accounts/{}/email/sending/send", self.url, self.account_id);
        let response = self.http.post(url).bearer_auth(&self.token).json(message).send().await.map_err(|error| {
            tracing::warn!(%error, "Email Sending unreachable");
            ApiError::unavailable("Email couldn't be sent right now")
        })?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() && body["success"] != false {
            let list = |key: &str| body["result"][key].as_array().map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()).unwrap_or_default();
            return Ok(Sent {
                message_id: body["result"]["message_id"].as_str().map(str::to_string),
                delivered: list("delivered"),
                queued: list("queued"),
                permanent_bounces: list("permanent_bounces"),
                suppressed: list("suppressed_recipients"),
            });
        }
        let code = body["errors"][0]["message"].as_str().unwrap_or_default().to_string();
        tracing::warn!(%status, %code, "Email Sending refused a message");
        Err(match status.as_u16() {
            400 if code.ends_with("email.too_big") => ApiError::too_large("The message is too large to send"),
            400 => ApiError::coded(StatusCode::BAD_REQUEST, "Email Sending refused the message", "refused"),
            429 => ApiError::unavailable("Email couldn't be sent right now").retry_after(60),
            _ => ApiError::unavailable("Email couldn't be sent right now"),
        })
    }
}

#[cfg(test)]
mod end_to_end;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_lowercase_letters_digits_dots_and_hyphens() {
        assert_eq!(check_name(" Egoist.Dev ").unwrap(), "egoist.dev");
        for bad in ["ab", "a..b", ".abc", "abc-", "a b c", "a+b", "名字字", &"a".repeat(33)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
        assert_eq!(check_name("Postmaster").unwrap_err().code(), Some("reserved"));
        let name = random_name();
        assert_eq!(check_name(&name).unwrap(), name);
        assert!(name.chars().next().unwrap().is_ascii_lowercase());
    }

    #[test]
    fn recipients_are_plain_addresses_off_the_mail_domain() {
        assert_eq!(check_recipient(" a@example.com ", "bots.lorca.app").unwrap(), "a@example.com");
        for bad in ["a", "a@b", "a@@b.com", "a@b.com\r\nBcc: x@y.com", "Name <a@b.com>", "a,b@c.com", "x@bots.lorca.app", "x@BOTS.lorca.app"] {
            assert!(check_recipient(bad, "bots.lorca.app").is_err(), "{bad}");
        }
    }
}
