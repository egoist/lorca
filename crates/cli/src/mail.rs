//! The account's email address. The relay hands it out on its mail domain, one per account and
//! only when the user asks, and every bot shares it: a bot writes from `name+<tag>@domain`, so a
//! reply comes back to that bot, and mail with no bot's tag goes to the lead bot. Every Device
//! asks the relay for the address when its sync socket opens and when the relay says it changed;
//! a Runner then tells the relay it takes the account's mail. Mail in reaches each Runner sealed
//! to its box key (`inbox`), and the Runner hosting the bot it is for keeps it and starts that
//! bot's turn; the bots read, wait for, and send mail with their tools (`tools`).

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::model::Bot;

#[cfg(feature = "runner")]
mod inbox;
#[cfg(feature = "runner")]
mod tools;

#[cfg(feature = "runner")]
pub use inbox::{finished, receive, recover, run, task_for_job, tick, JOB_KIND};
#[cfg(feature = "runner")]
pub use tools::{execute_reviewed, prompt_note, review_call, tools, SendEmail};

/// The id a bot's Access gives email under, as it gives a plugin.
pub const CONNECTION: &str = "email";
/// The sealed plaintext of a message the mail Worker delivered starts with this line.
pub const MAGIC: &[u8] = b"lorca-mail/1\n";

/// What the relay said about the account's address when this Device last asked.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Relayed {
    pub domain: String,
    pub name: Option<String>,
    pub suspended_until: Option<i64>,
    /// This machine takes the account's mail.
    pub runner: bool,
}

impl Relayed {
    fn from_relay(value: &Value) -> Self {
        Relayed {
            domain: value["domain"].as_str().unwrap_or_default().to_string(),
            name: value["address"]["name"].as_str().map(str::to_string),
            suspended_until: value["address"]["suspended_until"].as_i64(),
            runner: value["runner"].as_bool().unwrap_or(false),
        }
    }

    pub fn email(&self, tag: Option<&str>) -> Option<String> {
        let name = self.name.as_deref()?;
        Some(match tag {
            Some(tag) => format!("{name}+{tag}@{}", self.domain),
            None => format!("{name}@{}", self.domain),
        })
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended_until.is_some_and(|until| until > crate::config::now_unix())
    }
}

/// This Device's view of the account's email, kept in `lorca.sqlite3` so a Runner that starts
/// without the relay still knows its bots' addresses.
#[derive(Default)]
pub struct Mail {
    /// `None` until the relay was asked; `Some(None)` when it has no email.
    relayed: Mutex<Option<Option<Relayed>>>,
    #[cfg(feature = "runner")]
    pub(crate) waiters: Mutex<Vec<inbox::Waiter>>,
}

impl Mail {
    pub fn load(store: &crate::local_store::LocalStore) -> Self {
        let relayed = store.mail_status().ok().flatten().and_then(|json| serde_json::from_str(&json).ok());
        Mail { relayed: Mutex::new(relayed), ..Mail::default() }
    }
}

/// The address and domain, when the account has one.
pub fn relayed(app: &App) -> Option<Relayed> {
    app.mail.relayed.lock().unwrap().clone().flatten().filter(|relayed| relayed.name.is_some())
}

fn remember(app: &App, relayed: Option<Relayed>) {
    let changed = {
        let mut current = app.mail.relayed.lock().unwrap();
        let changed = current.as_ref() != Some(&relayed);
        *current = Some(relayed.clone());
        changed
    };
    if changed {
        if let Err(error) = app.store.set_mail_status(&serde_json::to_string(&relayed).unwrap_or_default()) {
            tracing::warn!(%error, "keeping the account's email address");
        }
        app.emit(crate::events::Event::MailChanged(status(app)));
    }
}

// MARK: - Tags

/// `Scout` → `scout`, `Ops Lead` → `ops-lead`: lowercase ASCII letters and digits, anything
/// else a hyphen, at most 40 characters.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    slug[..slug.len().min(40)].trim_end_matches('-').to_string()
}

/// A bot's id without its prefix: the tag of a bot whose name gives none of its own.
fn id_tag(bot: &Bot) -> String {
    let id = bot.id.strip_prefix("bot-").unwrap_or(&bot.id).to_ascii_lowercase();
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect()
}

/// The `+tag` a bot writes from: its name, or its id when its name gives no tag or another
/// bot's name gives the same one.
pub fn tag(bots: &[Bot], bot: &Bot) -> String {
    let own = slug(&bot.name);
    if own.is_empty() || bots.iter().filter(|other| slug(&other.name) == own).count() > 1 || bots.iter().any(|other| id_tag(other) == own) {
        return id_tag(bot);
    }
    own
}

/// The bot a `+tag` names, by its name's tag or its id's.
pub fn bot_for_tag<'a>(bots: &'a [Bot], tag_text: &str) -> Option<&'a Bot> {
    let wanted = tag_text.to_ascii_lowercase();
    bots.iter().find(|bot| tag(bots, bot) == wanted).or_else(|| bots.iter().find(|bot| id_tag(bot) == wanted))
}

/// Who gets mail that names no bot: the coordinator the user picked for Attention, else the
/// account's first bot. Every Device reads the same roster, so every Runner picks the same one.
pub fn lead_bot(app: &App) -> Option<Bot> {
    let preferred = crate::attention::preferences(app).default_coordinator_bot_id;
    let state = app.state.lock().unwrap();
    preferred.and_then(|id| state.bots.iter().find(|bot| bot.id == id).cloned()).or_else(|| state.bots.first().cloned())
}

/// The bot's own address, `name+scout@domain`.
pub fn bot_address(app: &App, bot: &Bot) -> Option<String> {
    let relayed = relayed(app)?;
    let bots = app.state.lock().unwrap().bots.clone();
    relayed.email(Some(&tag(&bots, bot)))
}

// MARK: - The apps' view

/// `MailStatus` for the apps; `null` before the relay was asked.
pub fn status(app: &App) -> Value {
    let Some(relayed) = app.mail.relayed.lock().unwrap().clone() else { return Value::Null };
    let Some(relayed) = relayed else {
        return json!({ "available": false, "domain": null, "address": null, "lead_bot_id": null, "bots": [] });
    };
    let bots = app.state.lock().unwrap().bots.clone();
    let address = relayed.email(None).map(|email| json!({
        "name": relayed.name,
        "email": email,
        "state": if relayed.is_suspended() { "suspended" } else { "active" },
    }));
    let addresses: Vec<Value> = match relayed.name {
        Some(_) => bots.iter().filter_map(|bot| Some(json!({ "bot_id": bot.id, "email": relayed.email(Some(&tag(&bots, bot)))? }))).collect(),
        None => Vec::new(),
    };
    json!({
        "available": true,
        "domain": relayed.domain,
        "address": address,
        "lead_bot_id": address.as_ref().and_then(|_| lead_bot(app)).map(|bot| bot.id),
        "bots": addresses,
    })
}

/// The status again once the roster changed, which moves the bots' addresses and the lead bot.
pub fn status_if_address(app: &App) -> Option<Value> {
    relayed(app).map(|_| status(app))
}

// MARK: - The relay

async fn bearer(app: &Arc<App>) -> Result<(String, String), String> {
    let url = app.relay_url().ok_or("No relay configured")?;
    let machine_file = app.machine_file().ok_or("Create or pair an identity first")?;
    let machine = machine_file.machine().map_err(|error| error.to_string())?;
    let token = crate::sync::token_or_register(app, &url, &machine).await.map_err(|error| error.message)?;
    Ok((url, token))
}

/// Asks the relay for the account's address, and as a Runner, tells it this machine takes the
/// account's mail once there is one. A relay without email answers `404`.
pub async fn refresh(app: &Arc<App>) -> Result<(), String> {
    let (url, token) = bearer(app).await?;
    let answer = app.relay.mail(reqwest::Method::GET, &url, &token, "/v1/mail/address", None).await;
    let relayed = match answer {
        Ok(value) => Some(Relayed::from_relay(&value)),
        Err(error) if error.status == Some(404) => None,
        Err(error) => return Err(error.message),
    };
    let relayed = match relayed {
        Some(relayed) if cfg!(feature = "runner") && relayed.name.is_some() && !relayed.runner => {
            let value = app.relay.mail(reqwest::Method::PUT, &url, &token, "/v1/mail/runner", None).await.map_err(|error| error.message)?;
            Some(Relayed::from_relay(&value))
        }
        other => other,
    };
    remember(app, relayed);
    Ok(())
}

/// `refresh` from the sync loop, which goes on whatever the relay answers.
pub async fn refresh_quietly(app: Arc<App>) {
    if let Err(error) = refresh(&app).await {
        tracing::debug!(%error, "asking the relay for the account's email address");
    }
}

/// The local API: `mail.get`, `mail.apply { name? }`, `mail.release`.
pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "mail.get" => {
            refresh(app).await?;
            Ok(status(app))
        }
        "mail.apply" => {
            let body = match params["name"].as_str().map(|name| name.trim().to_ascii_lowercase()).filter(|name| !name.is_empty()) {
                Some(name) => json!({ "name": name }),
                None => json!({}),
            };
            let (url, token) = bearer(app).await?;
            match app.relay.mail(reqwest::Method::POST, &url, &token, "/v1/mail/address", Some(&body)).await {
                Ok(_) => {}
                Err(error) if matches!(error.code.as_deref(), Some("invalid" | "reserved")) => return Ok(json!({ "problem": error.code })),
                Err(error) if error.code.as_deref() == Some("limit") => return Err("This account has changed its address as many times as it may.".into()),
                Err(error) if error.status == Some(409) => return Ok(json!({ "problem": "taken" })),
                Err(error) if error.status == Some(404) => return Err("This relay has no email".into()),
                Err(error) => return Err(error.message),
            }
            refresh(app).await?;
            Ok(json!({ "mail": status(app) }))
        }
        "mail.release" => {
            let (url, token) = bearer(app).await?;
            app.relay.mail(reqwest::Method::DELETE, &url, &token, "/v1/mail/address", None).await.map_err(|error| error.message)?;
            refresh(app).await?;
            Ok(json!({ "mail": status(app) }))
        }
        other => Err(format!("Unknown method {other}")),
    }
}

/// The catalog entry for a bot's Access sheet, beside the Runner's plugins, while the account
/// has an address.
pub fn access_entry(app: &App) -> Option<Value> {
    relayed(app)?;
    Some(json!({
        "id": CONNECTION,
        "name": "Email",
        "tools": [
            { "name": "email", "title": "Read and wait for email", "description": "Read, search, and wait for mail to the bot's address.", "read_only": true, "capability": "read", "hidden": false },
            { "name": "send_email", "title": "Send email", "description": "Send mail and reply from the bot's address.", "read_only": false, "capability": "write", "hidden": false },
        ],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot(id: &str, name: &str) -> Bot {
        serde_json::from_value(json!({ "id": id, "name": name, "description": "", "symbol_name": "sparkles", "accent": "blue", "runner_id": "runner", "provider": "deepseek", "created_at": 0 })).unwrap()
    }

    #[test]
    fn a_bot_writes_from_its_name_or_its_id() {
        let bots = vec![bot("bot-1a2b3c4d", "Scout"), bot("bot-5e6f7a8b", "Ops Lead!"), bot("bot-9c0d1e2f", "研究员"), bot("bot-0a0b0c0d", "scout")];
        assert_eq!(tag(&bots, &bots[1]), "ops-lead");
        assert_eq!(tag(&bots, &bots[2]), "9c0d1e2f", "a name with no letters of its own gives the id");
        assert_eq!(tag(&bots, &bots[0]), "1a2b3c4d", "two bots named alike both use their ids");
        assert_eq!(bot_for_tag(&bots, "OPS-LEAD").map(|b| b.id.as_str()), Some("bot-5e6f7a8b"));
        assert_eq!(bot_for_tag(&bots, "0a0b0c0d").map(|b| b.id.as_str()), Some("bot-0a0b0c0d"));
        assert_eq!(bot_for_tag(&bots, "scout"), None, "a tag two bots share names neither");
        assert_eq!(slug(&"x".repeat(60)).len(), 40);
    }

    /// The mail Worker seals with `@noble` (`mail/src/seal.ts`); its test writes this vector, and
    /// a Runner's box secret opens it.
    #[test]
    fn a_runner_opens_what_the_mail_worker_seals() {
        let vector: Value = serde_json::from_str(include_str!("../../../mail/test/seal-vector.json")).unwrap();
        let field = |key: &str| crate::keys::unb64(vector[key].as_str().unwrap()).unwrap();
        let secret = crypto_box::SecretKey::from_bytes(field("box_secret").try_into().unwrap());
        assert_eq!(secret.public_key().as_bytes().to_vec(), field("box_public"));
        let opened = crate::crypto::unseal(&secret, &field("sealed")).unwrap();
        assert_eq!(opened, field("plaintext"));
        assert!(opened.starts_with(MAGIC));
    }

    #[test]
    fn an_address_has_the_bots_tag_after_a_plus() {
        let relayed = Relayed { domain: "bots.lorca.app".into(), name: Some("k7f3m9q2".into()), suspended_until: None, runner: false };
        assert_eq!(relayed.email(None).unwrap(), "k7f3m9q2@bots.lorca.app");
        assert_eq!(relayed.email(Some("scout")).unwrap(), "k7f3m9q2+scout@bots.lorca.app");
        assert!(!relayed.is_suspended());
    }
}
