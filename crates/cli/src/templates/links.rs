//! Templates shared as links. The template goes up encrypted with a key of its own, under a
//! random id, to the relay this Device syncs with; the key travels only in the link, after its
//! `#`, which no browser sends, so neither the relay nor lorca.app can read the template. The
//! account keeps each link (id, key, relay, and what it holds) in its roster, so every Device
//! lists, updates, and revokes it.

use std::sync::Arc;

use rand::RngCore;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::Selection;
use crate::app::App;
use crate::keys::{b64, unb64_32};

/// Where a link's page lives; the apps open `lorca://` and `lorca-dev://` links of the same shape.
pub const SITE: &str = "https://lorca.app";
/// The relay a link names by leaving it out.
pub const PUBLIC_RELAY: &str = "https://relay.lorca.app";
/// The associated data of a link's envelope.
const KIND: &str = "template";

/// A bot shared as a link.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SharedLink {
    pub id: String,
    /// The template's key, base64url: the link's fragment.
    pub key: String,
    /// The relay that holds the ciphertext.
    pub relay: String,
    pub bot_id: String,
    /// The bot's name when it was last shared, for a bot deleted since.
    pub name: String,
    /// What the link holds, for its next update.
    pub selection: Selection,
    pub created_at: f64,
    pub updated_at: f64,
}

impl SharedLink {
    pub fn url(&self) -> String {
        let mut url = Url::parse(&format!("{SITE}/t/{}", self.id)).expect("site URL");
        if self.relay != PUBLIC_RELAY {
            url.query_pairs_mut().append_pair("relay", &self.relay);
        }
        url.set_fragment(Some(&self.key));
        url.to_string()
    }

    /// What the apps see of a link, in the snapshot and the roster events: the whole address,
    /// whose fragment is the key, and what it holds.
    pub fn out(&self) -> Value {
        json!({ "id": self.id, "url": self.url(), "bot_id": self.bot_id, "name": self.name, "selection": self.selection,
            "created_at": self.created_at, "updated_at": self.updated_at })
    }
}

/// A link's id, key, and relay, as lorca.app, the apps' `lorca://`, or a pasted address spells it.
#[derive(Debug)]
pub struct Parsed {
    pub id: String,
    pub key: [u8; 32],
    pub relay: String,
}

pub fn parse(link: &str) -> Result<Parsed, String> {
    let not_a_link = || "That isn't a link to a shared bot.".to_string();
    let url = Url::parse(link.trim()).map_err(|_| not_a_link())?;
    // `lorca://t/<id>` has `t` for its host; `https://lorca.app/t/<id>` has it in the path.
    let id = match url.scheme() {
        "http" | "https" => url.path().strip_prefix("/t/").map(str::to_string),
        "lorca" | "lorca-dev" if url.host_str() == Some("t") => url.path().strip_prefix('/').map(str::to_string),
        _ => None,
    }
    .filter(|id| !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .ok_or_else(not_a_link)?;
    let relay = match url.query_pairs().find(|(name, _)| name == "relay") {
        Some((_, relay)) => {
            let relay = Url::parse(&relay).map_err(|_| not_a_link())?;
            if !matches!(relay.scheme(), "http" | "https") || relay.host_str().is_none() {
                return Err(not_a_link());
            }
            relay.as_str().trim_end_matches('/').to_string()
        }
        None => PUBLIC_RELAY.to_string(),
    };
    let key = url.fragment().filter(|key| !key.is_empty()).ok_or("This link is missing its key, the part after #. Ask for the whole link.")?;
    let key = unb64_32(key).map_err(|_| "This link's key is cut short. Ask for the whole link.".to_string())?;
    Ok(Parsed { id, key, relay })
}

/// The template a link holds, as the file's text.
pub async fn fetch(app: &Arc<App>, link: &str) -> Result<String, String> {
    let link = parse(link)?;
    let ciphertext = app
        .relay
        .get_share(&link.relay, &link.id)
        .await
        .map_err(|error| format!("Couldn't reach {}: {}", host(&link.relay), error.message))?
        .ok_or("This link was revoked, or it never existed.")?;
    let text = crate::crypto::decrypt(&link.key, KIND, &ciphertext).map_err(|_| "This link's key doesn't open it. Ask for the whole link.".to_string())?;
    String::from_utf8(text).map_err(|_| "This link doesn't hold a bot template.".to_string())
}

/// Uploads the reviewed template under a new link, or under `link_id`'s, which keeps its address.
pub async fn share(app: &Arc<App>, params: &Value) -> Result<Value, String> {
    let bot_id = super::required(params, "bot_id")?;
    let selection: Selection = serde_json::from_value(params["selection"].clone()).map_err(|e| format!("Invalid export selection: {e}"))?;
    let preview = super::export_preview(app, bot_id, &selection).await?;
    super::review(params, &preview.digest)?;
    let text = serde_json::to_string_pretty(&preview.template).map_err(|e| e.to_string())?;
    let relay = app.relay_url().ok_or("Sharing a link needs a relay. Set one in Settings.")?;
    let existing = match params["link_id"].as_str() {
        Some(id) => Some(app.state.lock().unwrap().shared_links.iter().find(|link| link.id == id).cloned().ok_or("That link is gone. Share the bot again.")?),
        None => None,
    };
    let now = crate::config::now_secs();
    let mut link = existing.unwrap_or_else(|| {
        let mut id = [0u8; 16];
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut id);
        rand::rngs::OsRng.fill_bytes(&mut key);
        SharedLink { id: b64(&id), key: b64(&key), relay: relay.clone(), bot_id: bot_id.into(), name: String::new(), selection: Selection::default(), created_at: now, updated_at: now }
    });
    if link.relay != relay {
        return Err(format!("This link is on {}, and this Device syncs with {}. Share the bot again for a new link.", host(&link.relay), host(&relay)));
    }
    let key = unb64_32(&link.key).map_err(|e| e.to_string())?;
    let ciphertext = crate::crypto::encrypt(&key, KIND, text.as_bytes()).map_err(|e| e.to_string())?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("This Device isn't paired.")?;
    let token = crate::sync::token_or_register(app, &relay, &machine).await.map_err(|e| e.to_string())?;
    app.relay.put_share(&relay, &token, &link.id, ciphertext).await.map_err(|error| match error.status {
        Some(404) | Some(405) => format!("{} doesn't keep shared links yet. Update the relay.", host(&relay)),
        _ => error.message,
    })?;
    link.bot_id = bot_id.into();
    link.name = app.bot(bot_id).map(|bot| bot.name).unwrap_or_default();
    link.selection = selection;
    link.updated_at = now;
    {
        let mut state = app.state.lock().unwrap();
        match state.shared_links.iter_mut().find(|each| each.id == link.id) {
            Some(each) => *each = link.clone(),
            None => state.shared_links.push(link.clone()),
        }
    }
    app.roster_changed(true);
    Ok(json!({ "link": link.out() }))
}

/// Takes the link down: its address answers that it was revoked from now on.
pub async fn revoke(app: &Arc<App>, link_id: &str) -> Result<Value, String> {
    let link = app.state.lock().unwrap().shared_links.iter().find(|link| link.id == link_id).cloned().ok_or("That link is gone already.")?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("This Device isn't paired.")?;
    let token = crate::sync::token_or_register(app, &link.relay, &machine).await.map_err(|e| e.to_string())?;
    match app.relay.delete_share(&link.relay, &token, &link.id).await {
        Ok(()) => {}
        // Gone from the relay already: forgetting it here is what is left.
        Err(error) if error.status == Some(404) => {}
        Err(error) => return Err(error.message),
    }
    app.state.lock().unwrap().shared_links.retain(|each| each.id != link.id);
    app.roster_changed(true);
    Ok(Value::Null)
}

fn host(url: &str) -> String {
    Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(relay: &str) -> SharedLink {
        SharedLink { id: "AbC-_9".into(), key: b64(&[7u8; 32]), relay: relay.into(), bot_id: "bot".into(), name: "Bot".into(),
            selection: Selection::default(), created_at: 0.0, updated_at: 0.0 }
    }

    #[test]
    fn links_read_back_the_way_they_are_written() {
        let public = link(PUBLIC_RELAY);
        assert_eq!(public.url(), format!("https://lorca.app/t/AbC-_9#{}", public.key));
        let own = link("http://192.168.1.4:8787");
        assert!(own.url().starts_with("https://lorca.app/t/AbC-_9?relay=http%3A%2F%2F192.168.1.4%3A8787#"));
        for (spelling, relay) in [
            (public.url(), PUBLIC_RELAY),
            (own.url(), "http://192.168.1.4:8787"),
            (public.url().replace("https://lorca.app/t/", "lorca://t/"), PUBLIC_RELAY),
            (own.url().replace("https://lorca.app/t/", "lorca-dev://t/"), "http://192.168.1.4:8787"),
            (format!("  {}\n", public.url()), PUBLIC_RELAY),
        ] {
            let parsed = parse(&spelling).unwrap();
            assert_eq!((parsed.id.as_str(), parsed.key, parsed.relay.as_str()), ("AbC-_9", [7u8; 32], relay), "{spelling}");
        }
        assert!(parse("https://lorca.app/t/AbC-_9").unwrap_err().contains("missing its key"));
        assert!(parse("https://lorca.app/t/AbC-_9#short").unwrap_err().contains("cut short"));
        for not_a_link in ["https://lorca.app/docs", "lorca://pair/xyz#k", "ftp://lorca.app/t/a#k", "hello", "https://lorca.app/t/a?relay=file:///etc#k"] {
            assert!(parse(not_a_link).unwrap_err().contains("isn't a link"), "{not_a_link}");
        }
    }
}
