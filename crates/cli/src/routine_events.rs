//! Routines that run on events. Such a routine owns an event subscription on its Runner
//! ([`crate::event_triggers`]) aimed at it. A receiver the relay hosts, or a gateway the user
//! runs, turns a service's deliveries into events about one subject, each signed with the
//! subscription's secret and sealed to the Runner. Every event about the routine's subject
//! starts one run, deduplicated by its delivery id, and an event that says it `ends` makes its
//! run the routine's last. The core knows no service: which receiver and subject fit, the bot
//! reads in the service's plugin skill.
//!
//! A receiver that takes requests at an address of its own (`webhook`) answers the subscription
//! with that address, and the routine keeps it with the key the Runner made, whose hash alone
//! the relay holds.
//!
//! An event's payload, as receivers and gateways write it:
//! `{ "subject": "…", "kind": "…", "summary": "…", "title"?: "…", "url"?: "…", "ends"?: true,
//! "data"?: {…} }`. Everything in it came from outside and instructs nothing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::Routine;

#[cfg(feature = "runner")]
use {crate::app::App, crate::config::now_secs, serde_json::json, std::sync::Arc};

/// How a routine on events hears them.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Listening {
    /// Set up, and not yet subscribed on the relay (it was out of reach).
    #[default]
    Pending,
    /// The relay's receiver seals the subject's events to the Runner.
    Subscribed,
    /// The receiver needs the user first: an app installed on the service, an account
    /// connected. Its setup link comes from the relay.
    NeedsSetup,
    /// The relay has no such receiver: the events come through the user's gateway.
    Gateway,
}

/// What a routine on events listens to, as the roster carries it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutineEvents {
    /// The receiver on the relay: `github`.
    pub receiver: String,
    /// What its events are about, in the receiver's spelling: `acme/project#42`. Empty for a
    /// receiver whose events are the routine's own, such as its webhook's requests.
    #[serde(default)]
    pub subject: String,
    /// The event subscription on the bot's Runner that the events come in through.
    #[serde(default)]
    pub subscription_id: String,
    #[serde(default)]
    pub status: Listening,
    /// What the receiver calls its service: "GitHub".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_name: String,
    /// The subject's title and link, as the receiver or the latest event gave them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Where a sender posts its requests, for a receiver that takes them: the webhook's URL.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub endpoint: String,
    /// The key a sender includes as `Authorization: Bearer <key>`; the relay keeps its hash.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event: Option<LastEvent>,
    /// When an event said the subject ended: the run for it is the routine's last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<f64>,
}

/// The latest event, in the receiver's words: "Changes requested by kim".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LastEvent {
    pub summary: String,
    pub at: f64,
}

/// What a routine's subscription knows of what it listens to, so the Runner can tell it from
/// other subscriptions and clean it up when its routine is gone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReceiverSpec {
    pub receiver: String,
    pub subject: String,
    /// The subscription's row on the relay, once the receiver took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_id: Option<String>,
}

/// A receiver's name and a subject, checked: `github` and `acme/project#42`, or `webhook` and
/// none.
pub fn check(receiver: &str, subject: &str) -> Result<(String, String), String> {
    let receiver = receiver.trim().to_lowercase();
    if receiver.is_empty() || receiver.len() > 32 || !receiver.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')) {
        return Err(format!("{receiver:?} is not a receiver's name. The service's skill names it."));
    }
    let subject = subject.trim().to_string();
    if subject.chars().count() > 200 || subject.chars().any(char::is_control) {
        return Err("Give what the events are about, as the service's skill spells it.".into());
    }
    Ok((receiver, subject))
}

/// What starts the routine in words: "Watches acme/project#42", "When its webhook is called".
pub fn describe(events: &RoutineEvents) -> String {
    if !events.subject.is_empty() {
        format!("Watches {}", events.subject)
    } else if events.receiver == "webhook" || !events.endpoint.is_empty() {
        "When its webhook is called".into()
    } else {
        format!("On {} events", events.receiver)
    }
}

/// A new key for a sender, and the hash the relay keeps of it.
pub fn new_key() -> (String, String) {
    let key = crate::keys::b64(&crate::keys::random_32());
    let hash = key_hash(&key);
    (key, hash)
}

/// SHA-256 of the key, in base64url: what the relay compares a request's key with.
pub fn key_hash(key: &str) -> String {
    use sha2::Digest;
    crate::keys::b64(&sha2::Sha256::digest(key.as_bytes()))
}

/// What a routine on events keeps of the Runner's view when another Device writes the roster
/// from an older copy: everything it listens with is the Runner's to write (how it listens, the
/// subject's title and link, the webhook's address and key, the latest event, its end), so the
/// Runner's copy stands. True when it kept any.
pub fn keep_reads(held: &Routine, incoming: &mut Routine) -> bool {
    let (Some(held), Some(events)) = (&held.events, incoming.events.as_mut()) else { return false };
    if (&held.receiver, &held.subject) != (&events.receiver, &events.subject) || held == events {
        return false;
    }
    let ended_at = events.ended_at.or(held.ended_at);
    *events = RoutineEvents { ended_at, ..held.clone() };
    true
}

/// The line the run's note opens with, from an event's own words; an event that ends its
/// subject says the run is the routine's last.
pub fn lead(payload: &Value) -> Option<String> {
    let summary = payload["summary"].as_str().map(str::trim).filter(|text| !text.is_empty())?;
    let subject = payload["subject"].as_str().unwrap_or_default();
    let mut lead = if subject.is_empty() { format!("Event: {summary}.") } else { format!("Event on {subject}: {summary}.") };
    if payload["ends"].as_bool() == Some(true) {
        lead.push_str(" It ends what this routine watches: this is the routine's last run.");
    }
    Some(lead)
}

// MARK: - On the Runner

/// Sets up the subscription of a routine on events on this Runner: events about its subject
/// start the routine's task in the bot's DM, held while the routine is paused.
#[cfg(feature = "runner")]
pub fn open(app: &Arc<App>, routine: &Routine) -> Result<String, String> {
    let events = routine.events.as_ref().ok_or("Not a routine on events")?;
    let config = crate::event_triggers::SubscriptionConfig {
        name: routine.name.clone(),
        source: "gateway_hmac".into(),
        bot_id: routine.bot_id.clone(),
        routine_id: Some(routine.id.clone()),
        prompt: routine.prompt.clone(),
        event_types: vec![events.receiver.clone()],
        // Events about the subject; with none, every event the receiver delivers here.
        filters: (!events.subject.is_empty()).then(|| crate::event_triggers::Filter { pointer: "/subject".into(), equals: Value::String(events.subject.clone()) }).into_iter().collect(),
        queue_policy: crate::event_triggers::QueuePolicy::Fifo,
        is_enabled: true,
        expires_at: None,
        channel: None,
        receiver: Some(ReceiverSpec { receiver: events.receiver.clone(), subject: events.subject.clone(), relay_id: None }),
    };
    let created = crate::event_triggers::serve(app, "events.create", &json!({ "config": config })).map_err(|error| error.to_string())?;
    created["id"].as_str().map(str::to_string).ok_or_else(|| "The routine's subscription has no id".into())
}

/// What the relay said to a routine on events, for the bot.
#[cfg(feature = "runner")]
#[derive(Debug, Clone, PartialEq)]
pub enum Subscribed {
    Subscribed,
    /// The link the user follows first.
    NeedsSetup(String),
    Gateway,
    /// The relay could not be reached; the Runner tries again.
    Unreachable(String),
    /// The receiver refused the subject, in its own words: nothing to listen to.
    Refused(String),
}

/// Subscribes a routine's subject with the relay's receiver: the receiver seals its events to
/// this Runner, signed with the subscription's secret. A relay without the receiver leaves the
/// routine to the user's gateway. The answer is kept on the routine.
#[cfg(feature = "runner")]
pub async fn subscribe(app: &Arc<App>, routine_id: &str) -> Subscribed {
    let Some(events) = app.routine(routine_id).and_then(|routine| routine.events) else { return Subscribed::Refused("Not a routine on events".into()) };
    let (secret, generation) = match crate::event_triggers::secret_of(app, &events.subscription_id) {
        Ok(found) => found,
        Err(error) => return Subscribed::Refused(error.to_string()),
    };
    let Some(relay) = app.relay_url() else { return record(app, routine_id, Listening::Gateway, &Value::Null, Subscribed::Gateway) };
    let Some(machine) = app.machine_file().and_then(|m| m.machine().ok()) else { return Subscribed::Unreachable("This Device isn't paired.".into()) };
    let token = match crate::sync::token_or_register(app, &relay, &machine).await {
        Ok(token) => token,
        Err(error) => return Subscribed::Unreachable(error.to_string()),
    };
    // A receiver that takes requests checks the key's hash; the key stays with the routine.
    let (key, hash) = if events.key.is_empty() { new_key() } else { (events.key.clone(), key_hash(&events.key)) };
    let body = json!({ "subject": events.subject, "subscription_id": events.subscription_id, "generation": generation, "secret": secret, "key_hash": hash });
    let answer = match app.relay.receiver_subscribe(&relay, &token, &events.receiver, &body).await {
        Ok(answer) => answer,
        // A relay without this receiver: a self-hosted one, or one not set up for the service.
        Err(error) if error.status == Some(404) => return record(app, routine_id, Listening::Gateway, &Value::Null, Subscribed::Gateway),
        Err(error) if error.is_client_error() => return Subscribed::Refused(error.message),
        Err(error) => return Subscribed::Unreachable(error.message),
    };
    match answer["status"].as_str() {
        Some("subscribed") => {
            if let Err(error) = crate::event_triggers::set_relay_id(app, &events.subscription_id, answer["id"].as_str().map(str::to_string)) {
                return Subscribed::Unreachable(error.to_string());
            }
            if answer["endpoint"].as_str().is_some_and(|url| !url.is_empty()) {
                let _ = app.update_routine(routine_id, |routine| {
                    if let Some(events) = routine.events.as_mut() {
                        events.key = key.clone();
                    }
                });
            }
            record(app, routine_id, Listening::Subscribed, &answer, Subscribed::Subscribed)
        }
        Some("needs_setup") => {
            let url = answer["setup_url"].as_str().unwrap_or_default().to_string();
            record(app, routine_id, Listening::NeedsSetup, &answer, Subscribed::NeedsSetup(url))
        }
        Some("refused") => Subscribed::Refused(answer["message"].as_str().unwrap_or("The receiver refused it.").to_string()),
        _ => Subscribed::Unreachable(format!("The relay answered {answer}")),
    }
}

/// Keeps how the routine listens and what the receiver said of its subject.
#[cfg(feature = "runner")]
fn record(app: &App, routine_id: &str, status: Listening, answer: &Value, said: Subscribed) -> Subscribed {
    let text = |key: &str| answer[key].as_str().filter(|text| !text.is_empty()).map(str::to_string);
    let _ = app.update_routine(routine_id, |routine| {
        if let Some(events) = routine.events.as_mut() {
            events.status = status;
            if let Some(name) = text("name") {
                events.source_name = name;
            }
            if let Some(title) = text("title") {
                events.title = title;
            }
            if let Some(url) = text("url") {
                events.url = url;
            }
            if let Some(endpoint) = text("endpoint") {
                events.endpoint = endpoint;
            }
        }
    });
    said
}

/// Makes a new key for a routine's webhook and has the relay take its hash: the old key stops
/// working at once.
#[cfg(feature = "runner")]
pub async fn regenerate_key(app: &Arc<App>, routine_id: &str) -> Result<String, String> {
    let events = app.routine(routine_id).and_then(|routine| routine.events).ok_or("Unknown routine")?;
    if events.endpoint.is_empty() {
        return Err("This routine has no webhook.".into());
    }
    let relay_id = crate::event_triggers::receiver_spec(app, &events.subscription_id).map_err(|e| e.to_string())?.and_then(|spec| spec.relay_id).ok_or("The webhook isn't set up on the relay yet.")?;
    let relay = app.relay_url().ok_or("This Device has no relay.")?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("This Device isn't paired.")?;
    let token = crate::sync::token_or_register(app, &relay, &machine).await.map_err(|e| e.to_string())?;
    let (key, hash) = new_key();
    app.relay.receiver_key(&relay, &token, &events.receiver, &relay_id, &hash).await.map_err(|error| error.message)?;
    app.update_routine(routine_id, |routine| {
        if let Some(events) = routine.events.as_mut() {
            events.key = key.clone();
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(key)
}

/// What the bot hears about a routine on events it set up.
#[cfg(feature = "runner")]
pub fn told(events: &RoutineEvents, answer: &Subscribed) -> String {
    let subject = &events.subject;
    match answer {
        Subscribed::Subscribed if subject.is_empty() => {
            "Its webhook is ready: the user copies its URL, key, and Authorization header from the routine in the app, and each request a service sends there runs this routine once, with what it sent.".into()
        }
        Subscribed::Subscribed => format!("Listening for events on {subject}: each one starts a run in this chat, until one says it ends."),
        Subscribed::NeedsSetup(url) => {
            format!("The relay's {} receiver needs the user first for {subject}, so the routine waits. Give the user this link: {url} It starts on its own once that's done.", events.receiver)
        }
        Subscribed::Gateway => format!(
            "This relay has no {} receiver, so the events need the user's own gateway: `lorca events route {} <file>` exports its route, and the service's skill says how to run the gateway for it. Tell the user the steps.",
            events.receiver, events.subscription_id
        ),
        Subscribed::Unreachable(why) => format!("The relay couldn't be reached ({why}); the routine subscribes on its own once it can."),
        Subscribed::Refused(why) => why.clone(),
    }
}

/// Subscribes again, every two minutes in the background, the routines still waiting on the
/// relay or on the user's setup, which may be done since.
#[cfg(feature = "runner")]
pub fn retry_waiting(app: &Arc<App>, mine: &[Routine]) {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static TRIED: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    let now = crate::config::now_unix();
    for routine in mine {
        let Some(events) = &routine.events else { continue };
        if !matches!(events.status, Listening::Pending | Listening::NeedsSetup) || events.subscription_id.is_empty() || events.ended_at.is_some() {
            continue;
        }
        {
            let mut tried = TRIED.get_or_init(Default::default).lock().unwrap();
            if tried.get(&routine.id).is_some_and(|at| now - at < 120) {
                continue;
            }
            tried.insert(routine.id.clone(), now);
        }
        let (app, id) = (app.clone(), routine.id.clone());
        tokio::spawn(async move {
            subscribe(&app, &id).await;
        });
    }
}

/// Removes what a routine on events set up: its subscription here and its row on the relay.
#[cfg(feature = "runner")]
pub fn forget(app: &Arc<App>, subscription_id: &str) {
    let spec = crate::event_triggers::receiver_spec(app, subscription_id).ok().flatten();
    if let Err(error) = crate::event_triggers::remove_owned(app, subscription_id) {
        tracing::warn!(%error, "removing a routine's subscription");
    }
    let Some((receiver, relay_id)) = spec.and_then(|spec| spec.relay_id.map(|id| (spec.receiver, id))) else { return };
    let app = app.clone();
    tokio::spawn(async move {
        let (Some(relay), Some(machine)) = (app.relay_url(), app.machine_file().and_then(|m| m.machine().ok())) else { return };
        let Ok(token) = crate::sync::token_or_register(&app, &relay, &machine).await else { return };
        if let Err(error) = app.relay.receiver_unsubscribe(&relay, &token, &receiver, &relay_id).await {
            tracing::warn!(error = %error.message, "removing a subscription on the relay");
        }
    });
}

/// Removes the subscriptions of routines on events whose routine is gone: deleted here, or on
/// another Device.
#[cfg(feature = "runner")]
pub fn sweep(app: &Arc<App>) {
    let Ok(owned) = crate::event_triggers::receiver_subscriptions(app) else { return };
    for (id, routine_id) in owned {
        let alive = routine_id.as_deref().and_then(|routine_id| app.routine(routine_id)).is_some_and(|routine| routine.events.as_ref().is_some_and(|events| events.subscription_id == id));
        if !alive {
            forget(app, &id);
        }
    }
}

/// An event's turn of a routine on events ended: the routine keeps it as its run and as the
/// latest event, and the run of an event that ends the subject ends the routine.
#[cfg(feature = "runner")]
pub fn event_finished(app: &Arc<App>, routine_id: &str, payload: &str, outcome: crate::runtime::TurnOutcome) {
    let payload: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let text = |key: &str| payload[key].as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
    let ends = payload["ends"].as_bool() == Some(true);
    let outcome = match outcome {
        crate::runtime::TurnOutcome::Sent => "sent",
        crate::runtime::TurnOutcome::Pass => "pass",
        crate::runtime::TurnOutcome::Skipped => "error",
    };
    let _ = app.update_routine(routine_id, |routine| {
        let now = now_secs();
        routine.last_run_at = Some(now);
        routine.last_outcome = Some(outcome.into());
        if let Some(events) = routine.events.as_mut() {
            if let Some(summary) = text("summary") {
                events.last_event = Some(LastEvent { summary: summary.chars().take(200).collect(), at: now });
            }
            if let Some(title) = text("title") {
                events.title = title.chars().take(200).collect();
            }
            if let Some(url) = text("url").filter(|url| url.starts_with("https://")) {
                events.url = url;
            }
            if ends && events.ended_at.is_none() {
                events.ended_at = Some(now);
            }
            // The receiver no longer delivers (the service's app was removed): subscribe again,
            // which says what the user has to do.
            if payload["unsubscribed"].as_bool() == Some(true) {
                events.status = Listening::Pending;
            }
        }
    });
    crate::routines::end_if_finished(app, routine_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receivers_and_subjects_are_checked() {
        assert_eq!(check(" GitHub ", " acme/project#42 ").unwrap(), ("github".into(), "acme/project#42".into()));
        assert!(check("git hub", "x").is_err());
        assert_eq!(check("webhook", "  ").unwrap(), ("webhook".into(), String::new()), "a webhook's events need no subject");
        assert!(check("github", "a\nb").is_err());
    }

    #[test]
    fn an_events_own_words_open_its_run() {
        let lead = lead(&serde_json::json!({ "subject": "acme/project#42", "summary": "Merged", "ends": true })).unwrap();
        assert_eq!(lead, "Event on acme/project#42: Merged. It ends what this routine watches: this is the routine's last run.");
        assert_eq!(super::lead(&serde_json::json!({ "summary": "Comment by sam" })).unwrap(), "Event: Comment by sam.");
        assert_eq!(super::lead(&serde_json::json!({ "repository": "x" })), None);
    }
}
