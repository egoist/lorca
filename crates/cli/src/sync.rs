//! Relay sync: registration, auth, the outbox, presence, and applying incoming blobs.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::app::{remember_applied, upsert_device, App};
use crate::config::now_unix;
use crate::events::Event;
use crate::keys::unb64;
use crate::model::*;
use crate::relay::{BlobIn, RelayError};

const POLL_WAIT_SECS: u64 = 25;

pub async fn run(app: Arc<App>) {
    let mut failures: u32 = 0;
    loop {
        match cycle(&app).await {
            Ok(()) => failures = 0,
            Err(error) => {
                if app.relay_connected.swap(false, Ordering::Relaxed) {
                    app.emit(Event::RelayStatus { connected: false, url: app.relay_url() });
                }
                if error.is_unauthorized() {
                    app.relay.forget_token();
                }
                failures = failures.saturating_add(1);
                let delay = (2u64.pow(failures.min(5))).min(60);
                tracing::warn!(%error, retry_in = delay, "relay");
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {}
                    _ = app.outbox_notify.notified() => {}
                }
            }
        }
    }
}

async fn cycle(app: &Arc<App>) -> Result<(), RelayError> {
    let Some(machine_file) = app.machine_file() else {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        return Ok(());
    };
    let Some(url) = app.relay_url() else {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
            _ = app.outbox_notify.notified() => {}
        }
        return Ok(());
    };
    let machine = machine_file.machine().map_err(|e| RelayError { status: None, message: e.to_string() })?;

    if !machine_file.registered {
        ensure_registered(app, &url).await?;
    }

    let token = app.relay.token(&url, &machine).await?;
    if !app.relay_connected.swap(true, Ordering::Relaxed) {
        app.emit(Event::RelayStatus { connected: true, url: Some(url.clone()) });
    }

    app.push_machine_blob_if_changed();
    drain_outbox(app, &url, &token).await?;
    refresh_presence(app, &url, &token).await?;

    let since = app.state.lock().unwrap().last_seq;
    let poll = app.relay.list_blobs(&url, &token, since, "", POLL_WAIT_SECS);
    let (blobs, _head) = tokio::select! {
        result = poll => result?,
        _ = app.outbox_notify.notified() => return Ok(()),
    };

    for blob in blobs {
        apply_blob(app, &machine_file, &blob);
        let mut state = app.state.lock().unwrap();
        state.last_seq = state.last_seq.max(blob.seq);
    }
    app.save_state();
    Ok(())
}

/// Registers this machine with the relay. Only an identity device can sign that.
pub async fn ensure_registered(app: &Arc<App>, url: &str) -> Result<(), RelayError> {
    let identity = app
        .identity
        .lock()
        .unwrap()
        .clone()
        .and_then(|file| file.identity().ok())
        .ok_or_else(|| RelayError { status: None, message: "this Device is not registered and holds no identity key".into() })?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or_else(|| RelayError { status: None, message: "no machine".into() })?;
    app.relay.register(url, &identity, &machine.pubkey(), &machine.box_pubkey()).await?;
    if let Some(file) = app.machine.lock().unwrap().as_mut() {
        file.registered = true;
    }
    app.save_machine().map_err(|e| RelayError { status: None, message: e.to_string() })?;
    Ok(())
}

async fn drain_outbox(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(item) = app.state.lock().unwrap().outbox.first().cloned() else { return Ok(()) };
        match app.relay.put_blob(url, token, &item.id, &item.kind, item.recipient.as_deref(), &item.ciphertext).await {
            Ok(_) => {}
            Err(error) if error.is_client_error() && !error.is_unauthorized() => {
                tracing::warn!(%error, kind = %item.kind, "relay rejected blob; dropping");
            }
            Err(error) => return Err(error),
        }
        app.state.lock().unwrap().outbox.retain(|i| i.id != item.id);
        app.save_state();
    }
}

async fn refresh_presence(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    let (machines, _now) = app.relay.machines(url, token).await?;
    let changed = {
        let mut state = app.state.lock().unwrap();
        let before: Vec<bool> = state.devices.iter().map(|d| online(&state, &d.id)).collect();
        for machine in machines {
            state.device_seen.insert(machine.machine_pubkey.clone(), machine.last_seen);
            if let Some(device) = state.devices.iter_mut().find(|d| d.id == machine.machine_pubkey) {
                if device.box_pubkey.is_empty() {
                    device.box_pubkey = machine.box_pubkey.clone();
                }
            }
        }
        let after: Vec<bool> = state.devices.iter().map(|d| online(&state, &d.id)).collect();
        before != after
    };
    if changed {
        app.emit(app.roster_summary());
    }
    Ok(())
}

fn online(state: &crate::app::State, id: &str) -> bool {
    state.device_seen.get(id).map(|seen| now_unix() - seen < crate::app::ONLINE_WINDOW_SECS).unwrap_or(false)
}

pub async fn delete_remote_blob(app: &Arc<App>, id: &str) {
    let (Some(url), Some(machine)) = (app.relay_url(), app.machine_file().and_then(|m| m.machine().ok())) else { return };
    if let Ok(token) = app.relay.token(&url, &machine).await {
        if let Err(error) = app.relay.delete_blob(&url, &token, id).await {
            tracing::debug!(%error, id, "deleting consumed job blob");
        }
    }
}

// MARK: - Applying blobs

pub fn apply_blob(app: &Arc<App>, machine_file: &crate::keys::MachineFile, blob: &BlobIn) {
    let already = {
        let mut state = app.state.lock().unwrap();
        if state.applied_blob_ids.iter().any(|id| id == &blob.id) {
            true
        } else {
            remember_applied(&mut state, &blob.id);
            false
        }
    };
    if already {
        return;
    }
    let Ok(ciphertext) = unb64(&blob.ciphertext) else { return };
    let Ok(dek) = machine_file.dek() else { return };

    match blob.kind.as_str() {
        "roster" => match crate::crypto::decrypt_json::<RosterBlob>(&dek, "roster", &ciphertext) {
            Ok(roster) => apply_roster(app, roster),
            Err(error) => tracing::warn!(%error, "roster blob"),
        },
        "chat" => match crate::crypto::decrypt_json::<ChatBlob>(&dek, "chat", &ciphertext) {
            Ok(op) => apply_chat_op(app, op),
            Err(error) => tracing::warn!(%error, "chat blob"),
        },
        "machine" => match crate::crypto::decrypt_json::<MachineBlob>(&dek, "machine", &ciphertext) {
            Ok(MachineBlob { device }) => {
                if app.this_device_id().as_deref() == Some(device.id.as_str()) {
                    return;
                }
                let changed = upsert_device(&mut app.state.lock().unwrap().devices, device);
                if changed {
                    app.save_state();
                    app.emit(app.roster_summary());
                }
            }
            Err(error) => tracing::warn!(%error, "machine blob"),
        },
        "job" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<Job>(&machine.box_secret, &ciphertext) {
                Ok(job) => {
                    crate::runtime::prime_names(app);
                    crate::runtime::spawn_local_job(app.clone(), job, Some(blob.id.clone()));
                }
                Err(error) => tracing::warn!(%error, "job envelope"),
            }
        }
        _ => {}
    }
}

fn apply_roster(app: &Arc<App>, roster: RosterBlob) {
    let removed: Vec<String>;
    {
        let mut state = app.state.lock().unwrap();
        let local_updated = state.chats.iter().map(|_| 0.0).fold(0.0, f64::max);
        let _ = local_updated;
        state.bots = roster.bots;
        let incoming_ids: Vec<String> = roster.chats.iter().map(|c| c.id.clone()).collect();
        removed = state.chats.iter().filter(|c| !incoming_ids.contains(&c.meta.id)).map(|c| c.meta.id.clone()).collect();
        state.chats.retain(|c| incoming_ids.contains(&c.meta.id));
        for meta in roster.chats {
            match state.chats.iter_mut().find(|c| c.meta.id == meta.id) {
                Some(chat) => chat.meta = meta,
                None => state.chats.push(Chat { meta, messages: Vec::new(), unread_count: 0 }),
            }
        }
    }
    for chat_id in removed {
        app.cancel_chat(&chat_id);
        app.emit(Event::ChatRemoved { chat_id });
    }
    crate::runtime::prime_names(app);
    app.roster_changed(false);
}

fn apply_chat_op(app: &Arc<App>, op: ChatBlob) {
    match op {
        ChatBlob::Upsert { message } => {
            let from_bot = matches!(message.author, Author::Bot { .. });
            let is_new;
            {
                let mut state = app.state.lock().unwrap();
                if !state.chats.iter().any(|c| c.meta.id == message.chat_id) {
                    // Roster not here yet: keep the message under a placeholder until it is.
                    state.chats.push(Chat {
                        meta: ChatMeta { id: message.chat_id.clone(), kind: "group".into(), title: Some("Chat".into()), bot_ids: vec![], is_pinned: false, created_at: message.created_at },
                        messages: Vec::new(),
                        unread_count: 0,
                    });
                }
                let chat = state.chats.iter_mut().find(|c| c.meta.id == message.chat_id).unwrap();
                is_new = !chat.messages.iter().any(|m| m.id == message.id);
                if is_new && from_bot && message.is_complete() {
                    chat.unread_count += 1;
                }
            }
            app.upsert_message(message, false);
            app.save_state();
            if is_new && from_bot {
                app.emit(app.roster_summary());
            }
        }
        ChatBlob::Remove { chat_id, message_id } => app.remove_message(&chat_id, &message_id, false),
        ChatBlob::ClearUnread { chat_id } => app.mark_read(&chat_id),
    }
}

/// Restore path: pull the account DEK the identity device sealed to the content key.
pub async fn fetch_dek(app: &Arc<App>, url: &str, identity: &crate::keys::Identity, machine: &crate::keys::Machine) -> Result<[u8; 32], String> {
    let token = app.relay.authenticate(url, machine).await.map_err(|e| e.to_string())?;
    let (blobs, _) = app.relay.list_blobs(url, &token, 0, "key", 0).await.map_err(|e| e.to_string())?;
    for blob in blobs.iter().rev() {
        let Ok(ciphertext) = unb64(&blob.ciphertext) else { continue };
        if let Ok(bytes) = crate::crypto::unseal(&identity.content_secret, &ciphertext) {
            if let Ok(dek) = <[u8; 32]>::try_from(bytes.as_slice()) {
                return Ok(dek);
            }
        }
    }
    Err("The relay has no account key for this identity. Create the identity on a Device that is online first.".into())
}
