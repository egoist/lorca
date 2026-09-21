//! Relay sync: registration, auth, the outbox, presence, and applying incoming blobs.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::app::{remember_applied, upsert_device, App};
use crate::events::Event;
use crate::keys::unb64;
use crate::model::*;
use crate::relay::{BlobIn, RelayError, Signal};

/// A pull page at least this long is applied as a backlog: see `App::bulk_sync`.
const BULK_BLOBS: usize = 20;

/// What a pull takes. `file` blobs are left out: a transcript fetches them by id when it
/// needs them, so a photo sent to one bot is not downloaded by every Device.
pub const POLL_KINDS: &str = "roster,chat,machine,credentials,job,job_cancel,job_result,request,response";

pub async fn run(app: Arc<App>) {
    let mut failures: u32 = 0;
    loop {
        match session(&app).await {
            Ok(()) => failures = 0,
            Err(error) => {
                disconnected(&app);
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
                // The relay no longer serves this build. The app says so, and the next try
                // waits: the answer stays the same until Lorca is updated or the relay changes.
                let outdated = error.is_update_required();
                if outdated && !app.relay_update_required.swap(true, Ordering::Relaxed) {
                    app.emit_relay_status();
                }
                failures = failures.saturating_add(1);
                let delay = if outdated { 900 } else { (2u64.pow(failures.min(5))).min(60) };
                tracing::warn!(%error, retry_in = delay, "relay");
                // Up to a second on top, so the Devices a relay restart dropped together do
                // not all come back in the same instant.
                let jitter = std::time::Duration::from_millis(rand::Rng::gen_range(&mut rand::thread_rng(), 0..1000));
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(delay) + jitter) => {}
                    _ = app.outbox_notify.notified() => {}
                }
            }
        }
    }
}

/// Without its own socket this Device knows nothing of the others' presence.
fn disconnected(app: &Arc<App>) {
    if app.relay_connected.swap(false, Ordering::Relaxed) {
        app.emit_relay_status();
    }
    let had_online = {
        let mut state = app.state.lock().unwrap();
        let had = !state.device_online.is_empty();
        state.device_online.clear();
        had
    };
    if had_online {
        app.emit(app.roster_summary());
    }
}

/// One sync socket, from connect to its end. The relay signals over it and carries no data:
/// `blobs` is answered with a pull, `machines` with a fresh machine list. The outbox wakes
/// the session too. `Ok` means the identity or the relay URL changed and the next session
/// starts from there; an error is the socket or a request failing.
async fn session(app: &Arc<App>) -> Result<(), RelayError> {
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

    // The socket opens before the first pull, so no blob lands unseen between the two.
    let token = token_or_register(app, &url, &machine).await?;
    let mut socket = app.relay.sync_socket(&url, &token).await?;
    let was_refused = app.relay_update_required.swap(false, Ordering::Relaxed);
    if !app.relay_connected.swap(true, Ordering::Relaxed) || was_refused {
        app.emit_relay_status();
    }

    let (mut pull, mut refresh) = (true, true);
    let mut credentials_due = true;
    loop {
        let same_machine = app.machine_file().is_some_and(|file| file.machine().is_ok_and(|m| m.pubkey() == machine.pubkey()));
        if !same_machine || app.relay_url().as_deref() != Some(url.as_str()) {
            disconnected(app);
            return Ok(());
        }
        // Armed before the outbox is read: a blob queued while this round runs wakes the wait
        // below instead of sitting there until the relay next speaks.
        let queued = app.outbox_notify.notified();
        tokio::pin!(queued);
        queued.as_mut().enable();

        let token = token_or_register(app, &url, &machine).await?;
        app.push_machine_blob_if_changed();
        drain_outbox(app, &url, &token).await?;
        drain_group_deletes(app, &url, &token).await?;
        drain_blob_deletes(app, &url, &token).await?;
        if refresh {
            refresh_presence(app, &url, &token).await?;
        }
        if pull {
            pull_blobs(app, &url, &token, &machine_file).await?;
            // Once per session, so a relay that refuses the kind is not asked in a loop.
            if std::mem::take(&mut credentials_due) {
                app.push_credentials_if_owed();
            }
        }
        if app.presence_stale.swap(false, Ordering::Relaxed) {
            refresh_presence(app, &url, &token).await?;
        }

        (pull, refresh) = tokio::select! {
            signal = socket.next() => match signal? {
                Signal::Blobs => (true, false),
                Signal::Machines => (false, true),
            },
            // The outbox, or `sync.wake` from a phone that came back to the foreground: its
            // socket may have died unnoticed, and a pull costs one request.
            _ = &mut queued => (true, false),
        };
    }
}

/// Everything a Device polls for but the messages.
const NOT_CHAT_KINDS: &str = "roster,machine,credentials,job,job_cancel,job_result,request,response";
/// How much of each chat a Device takes when it first syncs: what a bot's turn reads.
const FIRST_SYNC_MESSAGES: usize = 400;
/// Messages to a page when reading a chat backwards.
const OLDER_PAGE: usize = 100;

/// A Device's first sync. The log holds every message of the account, so it is not replayed:
/// the Device takes the rest of the log, which slots keep short, then the newest messages of
/// each chat, and follows the log from where it stood when this began. What landed meanwhile
/// comes again through the log and changes nothing. Older messages stay on the relay until
/// someone scrolls to them (`older_messages`).
async fn first_sync(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile) -> Result<FirstSync, RelayError> {
    app.bulk_sync.store(true, Ordering::Relaxed);
    let synced = first_sync_quietly(app, url, token, machine_file).await;
    app.bulk_sync.store(false, Ordering::Relaxed);
    if matches!(synced, Ok(FirstSync::Done)) {
        app.save_state_now();
        crate::runtime::prime_names(app);
        app.emit(Event::Snapshot(app.snapshot()));
    }
    synced
}

#[derive(PartialEq)]
enum FirstSync {
    Done,
    /// The relay's log is empty.
    NothingThere,
    /// The relay cannot page a chat: the caller replays the log.
    CannotPage,
}

async fn first_sync_quietly(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile) -> Result<FirstSync, RelayError> {
    // The relay keeps the latest roster alone, so the chats have their names and bots before
    // their messages land.
    let (mut since, mut head) = (0, None);
    loop {
        let (blobs, seq) = app.relay.list_blobs(url, token, since, NOT_CHAT_KINDS).await?;
        let head = *head.get_or_insert(seq);
        let Some(last) = blobs.last().map(|blob| blob.seq) else {
            if head == 0 {
                return Ok(FirstSync::NothingThere);
            }
            break;
        };
        for blob in &blobs {
            apply_blob(app, machine_file, blob);
        }
        since = last;
    }
    let chats: Vec<String> = app.state.lock().unwrap().chats.iter().map(|chat| chat.meta.id.clone()).collect();
    for chat_id in chats {
        let page = app.relay.group_page(url, token, &crate::model::relay_name(&chat_id), None, FIRST_SYNC_MESSAGES).await;
        let (slots, has_more) = match page {
            Ok(page) => page,
            Err(error) if matches!(error.status, Some(404 | 405)) => return Ok(FirstSync::CannotPage),
            Err(error) => return Err(error),
        };
        for blob in slots.iter().flat_map(|slot| &slot.blobs) {
            apply_blob(app, machine_file, blob);
        }
        let before = slots.first().map(|slot| slot.place).filter(|_| has_more);
        if let Err(error) = app.store.set_history_before(&chat_id, before) {
            tracing::error!(%error, %chat_id, "recording where the chat begins");
        }
        // A page ends early at its byte budget; a bot's turn wants the whole count.
        let mut here = slots.len();
        while here < FIRST_SYNC_MESSAGES && app.history_is_partial(&chat_id) {
            here += older_messages_from(app, url, token, machine_file, &chat_id).await?.max(1);
        }
    }
    app.state.lock().unwrap().last_seq = head.unwrap_or(0);
    Ok(FirstSync::Done)
}

/// One chat at a time reads backwards, so two askers do not both take the same page.
static READING_BACK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fetches the page of messages before the ones this Device has of a chat. Returns how many
/// landed; 0 when the chat is here whole.
pub async fn older_messages(app: &Arc<App>, chat_id: &str) -> Result<usize, String> {
    let url = app.relay_url().ok_or("No relay configured")?;
    let machine_file = app.machine_file().ok_or("This Device is not paired")?;
    let machine = machine_file.machine().map_err(|e| e.to_string())?;
    let token = token_or_register(app, &url, &machine).await.map_err(|e| e.message)?;
    older_messages_from(app, &url, &token, &machine_file, chat_id).await.map_err(|e| e.message)
}

async fn older_messages_from(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile, chat_id: &str) -> Result<usize, RelayError> {
    let _reading = READING_BACK.lock().await;
    let local = |error: anyhow::Error| RelayError { status: None, message: error.to_string() };
    let Some(before) = app.store.history_before(chat_id).map_err(local)? else { return Ok(0) };
    let (slots, has_more) = app.relay.group_page(url, token, &crate::model::relay_name(chat_id), Some(before), OLDER_PAGE).await?;
    let dek = machine_file.dek().map_err(local)?;
    // A slot's last blob is the message as it stands. A removal or a read mark is nothing to
    // show, and a read mark from back then must not clear what is unread now.
    let newest_first: Vec<Message> = slots
        .iter()
        .rev()
        .filter_map(|slot| slot.blobs.last())
        .filter_map(|blob| crate::crypto::decrypt_json::<ChatBlob>(&dek, "chat", &unb64(&blob.ciphertext).ok()?).ok())
        .filter_map(|op| match op {
            ChatBlob::Upsert { message } if message.chat_id == chat_id => Some(message),
            _ => None,
        })
        .collect();
    app.store.insert_older(&newest_first).map_err(local)?;
    let before = slots.first().map(|slot| slot.place).filter(|_| has_more);
    app.store.set_history_before(chat_id, before).map_err(local)?;
    Ok(newest_first.len())
}

/// Pulls the log from `last_seq` until a page comes back empty.
async fn pull_blobs(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile) -> Result<(), RelayError> {
    if app.state.lock().unwrap().last_seq == 0 && first_sync(app, url, token, machine_file).await? == FirstSync::CannotPage {
        // A relay that cannot page a chat: replay its log. It keeps the latest roster, which
        // in a replay comes after the messages, so that is taken first as a preview.
        let (blobs, _head) = app.relay.list_blobs(url, token, 0, "roster,machine").await?;
        for blob in blobs {
            apply_blob_contents(app, machine_file, &blob);
        }
    }
    loop {
        let since = app.state.lock().unwrap().last_seq;
        let (blobs, _head) = app.relay.list_blobs(url, token, since, POLL_KINDS).await?;
        if blobs.is_empty() {
            return Ok(());
        }
        // A page this long is a backlog (a fresh pair replays the history): apply it quietly
        // and tell the app once, instead of one event and one state write per message.
        let bulk = blobs.len() >= BULK_BLOBS;
        app.bulk_sync.store(bulk, Ordering::Relaxed);
        for blob in blobs {
            apply_blob(app, machine_file, &blob);
            let mut state = app.state.lock().unwrap();
            state.last_seq = state.last_seq.max(blob.seq);
        }
        app.bulk_sync.store(false, Ordering::Relaxed);
        app.save_state_now();
        if bulk {
            crate::runtime::prime_names(app);
            app.emit(Event::Snapshot(app.snapshot()));
        }
    }
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
    let again = app.machine.lock().unwrap().as_mut().is_some_and(|file| std::mem::replace(&mut file.registered, true));
    // A relay that had to be told about this machine has none of its blobs either.
    {
        let mut state = app.state.lock().unwrap();
        state.machine_blob_hash = None;
        state.credentials_uploaded = false;
        if again {
            // It numbers its log from one, so the place held in the old log means nothing.
            state.last_seq = 0;
        }
    }
    // A relay that knew this machine before and lost the account (a reset, or the identity
    // dropped for inactivity) gets back what a Device pairing or restoring needs: the DEK
    // sealed to the content key, and the roster. Messages stay on the Devices that have them.
    if again {
        if let Some(dek) = app.dek() {
            match crate::crypto::seal(&identity.content_pubkey(), &dek) {
                Ok(sealed) => {
                    app.push_blob("key", None, sealed);
                }
                Err(error) => tracing::error!(%error, "sealing the account key"),
            }
        }
        app.push_roster();
    }
    app.save_machine().map_err(|e| RelayError { status: None, message: e.to_string() })?;
    Ok(())
}

async fn drain_outbox(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(item) = app
            .store
            .first_outbox()
            .map_err(|error| RelayError { status: None, message: error.to_string() })?
        else {
            return Ok(());
        };
        match app.relay.put_blob(url, token, &item).await {
            Ok(_) => {}
            Err(error) if error.is_client_error() && !error.is_unauthorized() => {
                tracing::warn!(%error, kind = %item.kind, "relay rejected blob; dropping");
                if item.kind == "credentials" {
                    app.state.lock().unwrap().credentials_uploaded = false;
                }
            }
            Err(error) => return Err(error),
        }
        let snapshot = app.state.lock().unwrap().clone();
        app.store
            .remove_outbox_with_state(&item.id, &snapshot)
            .map_err(|error| RelayError { status: None, message: error.to_string() })?;
    }
}

/// Tells the relay to drop the blobs of the chats deleted here. A relay that refuses (one
/// without groups) is not asked again; one that is away is asked on the next cycle.
async fn drain_group_deletes(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(group) = app.state.lock().unwrap().group_deletes.first().cloned() else { return Ok(()) };
        match app.relay.delete_group(url, token, &group).await {
            Ok(()) => {}
            Err(error) if error.is_client_error() && !error.is_unauthorized() && !error.is_unpaired() => {
                tracing::warn!(%error, "relay refused a group delete; dropping");
            }
            Err(error) => return Err(error),
        }
        app.state.lock().unwrap().group_deletes.retain(|g| g != &group);
        app.save_state();
    }
}

/// Tells the relay to drop the `file` blobs of the avatars dropped here. One the relay does
/// not have (never uploaded, or deleted by another Device) is done; a relay that is away is
/// asked on the next cycle.
async fn drain_blob_deletes(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(id) = app.state.lock().unwrap().blob_deletes.first().cloned() else { return Ok(()) };
        match app.relay.delete_blob(url, token, &id).await {
            Ok(()) => {}
            Err(error) if error.is_client_error() && !error.is_unauthorized() && !error.is_unpaired() => {
                tracing::debug!(%error, id, "relay had no such avatar blob; dropping");
            }
            Err(error) => return Err(error),
        }
        app.state.lock().unwrap().blob_deletes.retain(|queued| queued != &id);
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
        state.device_online = machines.iter().filter(|m| m.online).map(|m| m.machine_pubkey.clone()).collect();
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
        state.device_online.remove(id);
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

/// Deletes the account on the relay: every Device, this one among them, is unpaired by it.
/// Unlike `revoke_self` this has to land, or the relay keeps the data the user asked gone.
pub async fn delete_identity(app: &Arc<App>) -> Result<(), String> {
    let Some(url) = app.relay_url() else { return Ok(()) };
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("This Device has no identity")?;
    let token = token_or_register(app, &url, &machine).await.map_err(|e| e.message)?;
    match app.relay.delete_identity(&url, &token).await {
        Ok(()) => Ok(()),
        // The relay already dropped this Device, with the account or without it.
        Err(error) if error.is_unpaired() => Ok(()),
        Err(error) => Err(error.message),
    }
}

fn online(state: &crate::app::State, id: &str) -> bool {
    state.device_online.contains(id)
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
    apply_blob_contents(app, machine_file, blob);
}

/// Applies a blob whether or not it was applied before, and leaves no record of it.
fn apply_blob_contents(app: &Arc<App>, machine_file: &crate::keys::MachineFile, blob: &BlobIn) {
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
        "credentials" => match crate::crypto::decrypt_json::<crate::credentials::Credentials>(&dek, "credentials", &ciphertext) {
            Ok(credentials) => app.apply_credentials(&credentials),
            Err(error) => tracing::warn!(%error, "credentials blob"),
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
        "job_cancel" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<JobCancel>(&machine.box_secret, &ciphertext) {
                Ok(cancel) => {
                    app.cancel_job(&cancel.job_id);
                    let app = app.clone();
                    let blob_id = blob.id.clone();
                    tokio::spawn(async move { delete_remote_blob(&app, &blob_id).await });
                }
                Err(error) => tracing::warn!(%error, "job cancellation envelope"),
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

fn apply_roster(app: &Arc<App>, mut roster: RosterBlob) {
    let removed: Vec<String>;
    let normalized_descriptions = roster.bots.iter_mut().fold(false, |changed, bot| bot.normalize_description() || changed);
    let normalized_auto_review = crate::app::normalize_auto_review_rules(&roster.bots, &mut roster.auto_review);
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
                None => state.chats.push(Chat { meta, unread_count: 0, usage: None, compactions: Vec::new() }),
            }
        }
    }
    let snapshot = app.state.lock().unwrap().clone();
    if let Err(error) = app.store.save_state_deleting_chats(&snapshot, &removed) {
        tracing::error!(%error, "saving synced roster");
    }
    for chat_id in removed {
        app.cancel_chat(&chat_id);
        app.emit(Event::ChatRemoved { chat_id });
    }
    crate::runtime::prime_names(app);
    app.roster_changed(normalized_descriptions || normalized_auto_review);
}

fn apply_chat_op(app: &Arc<App>, op: ChatBlob) {
    match op {
        ChatBlob::Upsert { message } => {
            #[cfg(feature = "runner")]
            let steering = (message.author == Author::You && app.message(&message.chat_id, &message.id).is_none())
                .then(|| message.clone());
            {
                let mut state = app.state.lock().unwrap();
                if !state.chats.iter().any(|c| c.meta.id == message.chat_id) {
                    // Roster not here yet: keep the message under a placeholder until it is.
                    state.chats.push(Chat {
                        meta: ChatMeta { id: message.chat_id.clone(), kind: "group".into(), title: Some("Chat".into()), bot_ids: vec![], owner_bot_id: None, is_pinned: false, created_at: message.created_at },
                        unread_count: 0,
                        usage: None,
                        compactions: Vec::new(),
                    });
                }
            }
            // The cycle saves state once after the page.
            app.upsert_message(message, false);
            #[cfg(feature = "runner")]
            if let Some(message) = steering {
                crate::turns::steer_message(app, &message);
            }
        }
        ChatBlob::Remove { chat_id, message_id } => app.remove_message(&chat_id, &message_id, false),
        ChatBlob::ClearUnread { chat_id } => app.mark_read(&chat_id, false),
    }
}

/// Restore path: pull the account DEK the identity device sealed to the content key.
pub async fn fetch_dek(app: &Arc<App>, url: &str, identity: &crate::keys::Identity, machine: &crate::keys::Machine) -> Result<[u8; 32], String> {
    let token = app.relay.authenticate(url, machine).await.map_err(|e| e.to_string())?;
    let (blobs, _) = app.relay.list_blobs(url, &token, 0, "key").await.map_err(|e| e.to_string())?;
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
