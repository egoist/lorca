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
/// A poll page at least this long is applied as a backlog: see `App::bulk_sync`.
const BULK_BLOBS: usize = 20;

/// What the poll takes. `file` blobs are left out: a transcript fetches them by id when it
/// needs them, so a photo sent to one bot is not downloaded by every Device.
pub const POLL_KINDS: &str = "roster,chat,machine,job,job_result,request,response";

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
                // Another Device unpaired this one: the relay is done with its key, so its
                // copy of the account goes. Onboarding is next.
                if error.is_unpaired() {
                    tracing::warn!("this Device was unpaired; forgetting the identity");
                    if let Err(error) = app.forget_identity() {
                        tracing::error!(%error, "forgetting the identity");
                    }
                    failures = 0;
                    continue;
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

    let token = token_or_register(app, &url, &machine).await?;
    if !app.relay_connected.swap(true, Ordering::Relaxed) {
        app.emit(Event::RelayStatus { connected: true, url: Some(url.clone()) });
    }

    app.push_machine_blob_if_changed();
    drain_outbox(app, &url, &token).await?;
    refresh_presence(app, &url, &token).await?;

    let since = app.state.lock().unwrap().last_seq;
    // The relay keeps the latest roster, so in a replay from the start it comes after the
    // messages. It is taken first, and the chats have their names and bots when those land.
    if since == 0 {
        let (blobs, _head) = app.relay.list_blobs(&url, &token, 0, "roster,machine", 0).await?;
        for blob in blobs {
            apply_blob(app, &machine_file, &blob);
        }
    }
    let poll = app.relay.list_blobs(&url, &token, since, POLL_KINDS, POLL_WAIT_SECS);
    let (blobs, _head) = tokio::select! {
        result = poll => result?,
        _ = app.outbox_notify.notified() => return Ok(()),
    };

    // A page this long is a backlog (a fresh pair replays the history): apply it quietly and
    // tell the app once, instead of one event and one state write per message.
    let bulk = blobs.len() >= BULK_BLOBS;
    app.bulk_sync.store(bulk, Ordering::Relaxed);
    for blob in blobs {
        apply_blob(app, &machine_file, &blob);
        let mut state = app.state.lock().unwrap();
        state.last_seq = state.last_seq.max(blob.seq);
    }
    app.bulk_sync.store(false, Ordering::Relaxed);
    app.save_state_now();
    if bulk {
        crate::runtime::prime_names(app);
        app.emit(Event::Snapshot(app.snapshot()));
    }
    if app.presence_stale.swap(false, Ordering::Relaxed) {
        refresh_presence(app, &url, &token).await?;
    }
    Ok(())
}

/// A bearer for this machine. When the relay does not know the machine (a relay other than
/// the one that attested it, or a reset one) and this Device holds the identity, it attests
/// itself again and retries.
pub async fn token_or_register(app: &Arc<App>, url: &str, machine: &crate::keys::Machine) -> Result<String, RelayError> {
    match app.relay.token(url, machine).await {
        Err(error) if error.is_unknown_machine() && app.is_identity_device() => {
            tracing::info!(url, "relay does not know this machine; attesting it again");
            ensure_registered(app, url).await?;
            app.relay.token(url, machine).await
        }
        result => result,
    }
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
    // A relay that had to be told about this machine has none of its blobs either.
    app.state.lock().unwrap().machine_blob_hash = None;
    app.save_machine().map_err(|e| RelayError { status: None, message: e.to_string() })?;
    Ok(())
}

async fn drain_outbox(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(item) = app.state.lock().unwrap().outbox.first().cloned() else { return Ok(()) };
        match app.relay.put_blob(url, token, &item).await {
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

/// The relay's machine list is the list of paired Devices: presence comes from it, and a
/// Device it no longer lists was unpaired, so it leaves the roster here too.
async fn refresh_presence(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    let (machines, _now) = app.relay.machines(url, token).await?;
    let this_id = app.this_device_id();
    let (changed, pruned) = {
        let mut state = app.state.lock().unwrap();
        let before: Vec<bool> = state.devices.iter().map(|d| online(&state, &d.id)).collect();
        for machine in &machines {
            state.device_seen.insert(machine.machine_pubkey.clone(), machine.last_seen);
            if let Some(device) = state.devices.iter_mut().find(|d| d.id == machine.machine_pubkey) {
                if device.box_pubkey.is_empty() {
                    device.box_pubkey = machine.box_pubkey.clone();
                }
            }
        }
        let count = state.devices.len();
        state.devices.retain(|d| Some(&d.id) == this_id.as_ref() || machines.iter().any(|m| m.machine_pubkey == d.id));
        let pruned = state.devices.len() != count;
        if pruned {
            state.device_seen.retain(|id, _| Some(id) == this_id.as_ref() || machines.iter().any(|m| &m.machine_pubkey == id));
        }
        let after: Vec<bool> = state.devices.iter().map(|d| online(&state, &d.id)).collect();
        (before != after, pruned)
    };
    if pruned {
        app.save_state();
    }
    if changed || pruned {
        app.emit(app.roster_summary());
    }
    Ok(())
}

/// Unpairs another Device: the relay drops its key, and it leaves this roster now rather
/// than on the next presence refresh. A machine the relay already forgot still leaves.
pub async fn unpair_device(app: &Arc<App>, id: &str) -> Result<(), String> {
    let url = app.relay_url().ok_or("Set a relay URL first.")?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("No identity on this Device")?;
    let token = token_or_register(app, &url, &machine).await.map_err(|e| e.to_string())?;
    match app.relay.revoke_machine(&url, &token, id).await {
        Ok(()) => {}
        Err(error) if error.is_unknown_machine() => {}
        Err(error) => return Err(error.to_string()),
    }
    {
        let mut state = app.state.lock().unwrap();
        state.devices.retain(|d| d.id != id);
        state.device_seen.remove(id);
    }
    app.save_state();
    app.emit(app.roster_summary());
    Ok(())
}

/// Before this Device forgets the identity, it asks the relay to drop its key, so the other
/// Devices see it leave instead of an offline ghost. Best effort: the relay may be away.
pub async fn revoke_self(app: &Arc<App>) {
    let (Some(url), Some(machine)) = (app.relay_url(), app.machine_file().and_then(|m| m.machine().ok())) else { return };
    let revoke = async {
        let token = app.relay.token(&url, &machine).await?;
        app.relay.revoke_machine(&url, &token, &machine.pubkey()).await
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), revoke).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "revoking this machine on the relay"),
        Err(_) => tracing::warn!("revoking this machine on the relay timed out"),
    }
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
                let mut state = app.state.lock().unwrap();
                // A key the last presence refresh did not list is either a Device that just
                // paired or one unpaired since, whose old blob must not bring it back. It
                // lands quietly, and the refresh at the end of the cycle confirms or prunes it.
                if !state.device_seen.contains_key(&device.id) {
                    upsert_device(&mut state.devices, device);
                    app.presence_stale.store(true, Ordering::Relaxed);
                    return;
                }
                let changed = upsert_device(&mut state.devices, device);
                drop(state);
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
        "job_result" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<JobResult>(&machine.box_secret, &ciphertext) {
                Ok(result) => crate::runtime::deliver_job_result(app, result),
                Err(error) => tracing::warn!(%error, "job result envelope"),
            }
        }
        "request" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<Request>(&machine.box_secret, &ciphertext) {
                Ok(request) => crate::requests::serve(app.clone(), request, blob.id.clone()),
                Err(error) => tracing::warn!(%error, "request envelope"),
            }
        }
        "response" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<Response>(&machine.box_secret, &ciphertext) {
                Ok(response) => crate::requests::deliver(app.clone(), response, blob.id.clone()),
                Err(error) => tracing::warn!(%error, "response envelope"),
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
        state.routines = roster.routines;
        state.auto_review = roster.auto_review;
        let incoming_ids: Vec<String> = roster.chats.iter().map(|c| c.id.clone()).collect();
        removed = state.chats.iter().filter(|c| !incoming_ids.contains(&c.meta.id)).map(|c| c.meta.id.clone()).collect();
        state.chats.retain(|c| incoming_ids.contains(&c.meta.id));
        for meta in roster.chats {
            match state.chats.iter_mut().find(|c| c.meta.id == meta.id) {
                Some(chat) => chat.meta = meta,
                None => state.chats.push(Chat { meta, messages: Vec::new(), unread_count: 0, usage: None, compactions: Vec::new() }),
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
                        meta: ChatMeta { id: message.chat_id.clone(), kind: "group".into(), title: Some("Chat".into()), bot_ids: vec![], owner_bot_id: None, is_pinned: false, created_at: message.created_at },
                        messages: Vec::new(),
                        unread_count: 0,
                        usage: None,
                        compactions: Vec::new(),
                    });
                }
                let chat = state.chats.iter_mut().find(|c| c.meta.id == message.chat_id).unwrap();
                is_new = !chat.messages.iter().any(|m| m.id == message.id);
                if is_new && from_bot && message.is_complete() {
                    chat.unread_count += 1;
                }
            }
            // The cycle saves state once after the page.
            app.upsert_message(message, false);
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
