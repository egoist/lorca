//! Pairing. The identity device (A) publishes a pairing string; the joining Device (B) posts a
//! sealed request to the relay's pairing mailbox; A attests B's machine and seals the account
//! key back to it.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::app::{upsert_device, App, PairingStatus, PendingPairing};
use crate::config::now_unix;
use crate::events::Event;
use crate::keys::{self, Machine, MachineFile};
use crate::model::{host_facts, Device, PairReply, PairRequest};

const PAIR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const POLL: std::time::Duration = std::time::Duration::from_millis(1500);

pub fn parse_pairing_string(text: &str) -> anyhow::Result<(String, String, String, String)> {
    let text = text.trim();
    let query = text
        .strip_prefix("lorca://pair?")
        .or_else(|| text.split_once("pair?").map(|(_, q)| q))
        .ok_or_else(|| anyhow::anyhow!("That is not a Lorca pairing string"))?;
    let mut relay = None;
    let mut id = None;
    let mut ek = None;
    let mut nonce = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key {
            "relay" => relay = Some(value),
            "id" => id = Some(value),
            "ek" => ek = Some(value),
            "n" => nonce = Some(value),
            _ => {}
        }
    }
    match (relay, id, ek, nonce) {
        (Some(relay), Some(id), Some(ek), Some(nonce)) => Ok((relay, id, ek, nonce)),
        _ => Err(anyhow::anyhow!("Pairing string is missing a field")),
    }
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 {
            if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A: create the mailbox and start waiting. Returns the nonce and the pairing string.
pub async fn start(app: Arc<App>) -> anyhow::Result<(String, String)> {
    let identity_file = app.identity.lock().unwrap().clone().ok_or_else(|| anyhow::anyhow!("Only the Device that holds the identity can pair others."))?;
    let identity = identity_file.identity()?;
    let url = app.relay_url().ok_or_else(|| anyhow::anyhow!("Set a relay URL first. Pairing runs through the relay."))?;
    let machine_file = app.machine_file().ok_or_else(|| anyhow::anyhow!("No machine"))?;
    let machine = machine_file.machine()?;
    if !machine_file.registered {
        crate::sync::ensure_registered(&app, &url).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let token = crate::sync::token_or_register(&app, &url, &machine).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let nonce = app.relay.pair_create(&url, &token).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let ephemeral = crypto_box::SecretKey::generate(&mut rand::rngs::OsRng);
    let ek = keys::b64(ephemeral.public_key().as_bytes());
    let pairing_string = format!(
        "lorca://pair?relay={}&id={}&ek={}&n={}",
        percent_encode(&url),
        percent_encode(&identity.pubkey()),
        percent_encode(&ek),
        percent_encode(&nonce)
    );

    let cancel = CancellationToken::new();
    app.pairings.lock().unwrap().insert(nonce.clone(), PendingPairing { ephemeral, status: PairingStatus::Waiting, cancel: cancel.clone() });

    let waiter = app.clone();
    let waiter_nonce = nonce.clone();
    tokio::spawn(async move {
        let result = tokio::select! {
            _ = cancel.cancelled() => Err("Pairing cancelled".to_string()),
            result = tokio::time::timeout(PAIR_TIMEOUT, wait_for_request(&waiter, &url, &waiter_nonce)) => match result {
                Ok(result) => result,
                Err(_) => Err("Nobody joined within ten minutes".into()),
            },
        };
        let mut pairings = waiter.pairings.lock().unwrap();
        if let Some(pending) = pairings.get_mut(&waiter_nonce) {
            pending.status = match result {
                Ok(device) => PairingStatus::Completed { device },
                Err(error) => PairingStatus::Failed { error },
            };
        }
    });

    Ok((nonce, pairing_string))
}

async fn wait_for_request(app: &Arc<App>, url: &str, nonce: &str) -> Result<Value, String> {
    let identity = app.identity.lock().unwrap().clone().and_then(|f| f.identity().ok()).ok_or("no identity")?;
    let machine_file = app.machine_file().ok_or("no machine")?;
    let machine = machine_file.machine().map_err(|e| e.to_string())?;
    loop {
        let token = crate::sync::token_or_register(app, url, &machine).await.map_err(|e| e.to_string())?;
        let request = match app.relay.pair_get_request(url, &token, nonce).await {
            Ok(request) => request,
            Err(error) if error.is_unauthorized() => {
                app.relay.forget_token();
                None
            }
            Err(error) => return Err(error.to_string()),
        };
        let Some(sealed) = request else {
            tokio::time::sleep(POLL).await;
            continue;
        };
        let ephemeral = {
            let pairings = app.pairings.lock().unwrap();
            let pending = pairings.get(nonce).ok_or("pairing vanished")?;
            crypto_box::SecretKey::from_bytes(pending.ephemeral.to_bytes())
        };
        let request: PairRequest = crate::crypto::unseal_json(&ephemeral, &sealed).map_err(|e| e.to_string())?;
        keys::verifying_key(&request.machine_pubkey).map_err(|_| "joining Device sent a bad key".to_string())?;
        if request.device.id != request.machine_pubkey {
            return Err("joining Device's metadata does not match its key".into());
        }

        // Attest B, then hand it the account key.
        app.relay.register(url, &identity, &request.machine_pubkey, &request.box_pubkey).await.map_err(|e| e.to_string())?;
        let reply = PairReply {
            identity_pubkey: identity.pubkey(),
            content_pubkey: identity.content_pubkey(),
            account_dek: machine_file.account_dek.clone(),
            relay_url: url.to_string(),
        };
        let sealed_reply = crate::crypto::seal_json(&request.box_pubkey, &reply).map_err(|e| e.to_string())?;
        app.relay.pair_post_reply(url, &token, nonce, &sealed_reply).await.map_err(|e| e.to_string())?;

        let mut device = request.device.clone();
        device.box_pubkey = request.box_pubkey.clone();
        device.updated_at = now_unix();
        {
            let mut state = app.state.lock().unwrap();
            upsert_device(&mut state.devices, device.clone());
            state.device_seen.insert(device.id.clone(), now_unix());
            // It opens its sync socket next; the relay's `machines` signal confirms it.
            state.device_online.insert(device.id.clone());
            // The new Device needs this machine's metadata, whatever the relay already holds.
            state.machine_blob_hash = None;
        }
        app.push_machine_blob_if_changed();
        app.roster_changed(true);
        let out = json!({ "id": device.id, "name": device.name, "os": device.os, "model": device.model });
        app.emit(Event::PairCompleted { nonce: nonce.to_string(), device: out.clone() });
        return Ok(out);
    }
}

pub fn status(app: &Arc<App>, nonce: &str) -> Value {
    let pairings = app.pairings.lock().unwrap();
    match pairings.get(nonce) {
        Some(pending) => serde_json::to_value(&pending.status).unwrap_or(Value::Null),
        None => json!({ "state": "failed", "error": "Unknown pairing" }),
    }
}

/// A stops waiting. The mailbox on the relay goes too, so a Device still polling it learns
/// the code is dead instead of waiting out the TTL.
pub fn cancel(app: &Arc<App>, nonce: &str) {
    let Some(pending) = app.pairings.lock().unwrap().remove(nonce) else { return };
    pending.cancel.cancel();
    let app = app.clone();
    let nonce = nonce.to_string();
    tokio::spawn(async move {
        let Some(url) = app.relay_url() else { return };
        let Some(machine) = app.machine_file().and_then(|file| file.machine().ok()) else { return };
        if let Ok(token) = crate::sync::token_or_register(&app, &url, &machine).await {
            let _ = app.relay.pair_delete(&url, &token, &nonce).await;
        }
    });
}

/// B: join an identity with a pairing string from A. Waits for A's reply; `abort` ends the
/// wait, and a newer `accept` replaces one still waiting.
pub async fn accept(app: Arc<App>, pairing_string: &str, device_name: Option<String>) -> anyhow::Result<Value> {
    if app.has_identity() {
        anyhow::bail!("This Device already belongs to an identity.");
    }
    let cancel = CancellationToken::new();
    if let Some(previous) = app.accepting.lock().unwrap().replace(cancel.clone()) {
        previous.cancel();
    }
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(anyhow::anyhow!("Pairing cancelled")),
        result = join(&app, pairing_string, device_name) => result,
    };
    // Cancelled means a newer accept owns the slot, or abort already emptied it.
    if !cancel.is_cancelled() {
        app.accepting.lock().unwrap().take();
    }
    result
}

/// B gives up on the pairing it is waiting on.
pub fn abort(app: &Arc<App>) {
    if let Some(token) = app.accepting.lock().unwrap().take() {
        token.cancel();
    }
}

async fn join(app: &Arc<App>, pairing_string: &str, device_name: Option<String>) -> anyhow::Result<Value> {
    let (relay_url, identity_pubkey, ek, nonce) = parse_pairing_string(pairing_string)?;
    app.relay.health(&relay_url).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let machine = Machine::generate();
    let (host_name, os, os_version, model) = host_facts();
    let device = Device {
        id: machine.pubkey(),
        name: device_name.filter(|n| !n.trim().is_empty()).unwrap_or(host_name),
        model,
        os,
        os_version,
        box_pubkey: machine.box_pubkey(),
        plugins: Vec::new(),
        updated_at: now_unix(),
    };
    let request = PairRequest { machine_pubkey: machine.pubkey(), box_pubkey: machine.box_pubkey(), device: device.clone() };
    let sealed = crate::crypto::seal_json(&ek, &request)?;
    // A mailbox that is gone means A cancelled, or the code expired. One that already holds
    // a request was used by a Device already: a code is good for one pairing.
    let gone = |error: crate::relay::RelayError| match error.status {
        Some(404) => anyhow::anyhow!("The other Device stopped waiting on this code. Get a fresh one from it."),
        Some(409) => anyhow::anyhow!("This code was already used. Each code pairs one Device; get a fresh one from the other Device."),
        _ => anyhow::anyhow!("{error}"),
    };
    app.relay.pair_post_request(&relay_url, &nonce, &sealed).await.map_err(gone)?;
    app.emit(Event::PairPosted { nonce: nonce.clone() });

    let reply: PairReply = tokio::time::timeout(PAIR_TIMEOUT, async {
        loop {
            match app.relay.pair_get_reply(&relay_url, &nonce).await {
                Ok(Some(sealed)) => return crate::crypto::unseal_json::<PairReply>(&machine.box_secret, &sealed),
                Ok(None) => tokio::time::sleep(POLL).await,
                Err(error) => return Err(gone(error)),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("The other Device did not answer in time"))??;

    if reply.identity_pubkey != identity_pubkey {
        anyhow::bail!("The reply came from a different identity than the pairing string");
    }
    let dek = keys::unb64_32(&reply.account_dek)?;

    let machine_file = MachineFile {
        machine_secret: keys::b64(&machine.secret),
        identity_pubkey: reply.identity_pubkey.clone(),
        content_pubkey: reply.content_pubkey.clone(),
        account_dek: keys::b64(&dek),
        name: device.name.clone(),
        os: device.os.clone(),
        os_version: device.os_version.clone(),
        model: device.model.clone(),
        registered: true,
        relay_url: Some(reply.relay_url.clone()),
        created_at: now_unix(),
    };
    *app.machine.lock().unwrap() = Some(machine_file);
    app.save_machine()?;
    app.set_relay_url(Some(reply.relay_url.clone()))?;
    {
        let mut state = app.state.lock().unwrap();
        *state = Default::default();
        upsert_device(&mut state.devices, device.clone());
    }
    app.store.clear()?;
    app.save_state();
    app.push_machine_blob_if_changed();
    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(json!({ "id": device.id, "name": device.name, "os": device.os, "identity_id": keys::identity_id(&reply.identity_pubkey) }))
}
