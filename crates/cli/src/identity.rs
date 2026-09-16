//! Create or restore the identity on this Device.

use std::sync::Arc;

use crate::app::{upsert_device, App};
use crate::config::now_unix;
use crate::events::Event;
use crate::keys::{self, Identity, IdentityFile, Machine, MachineFile};
use crate::model::host_facts;

fn machine_file_for(identity_pubkey: &str, content_pubkey: &str, machine: &Machine, dek: &[u8; 32], registered: bool, relay_url: Option<String>, name: Option<String>) -> MachineFile {
    let (host_name, os, os_version, model) = host_facts();
    MachineFile {
        machine_secret: keys::b64(&machine.secret),
        identity_pubkey: identity_pubkey.to_string(),
        content_pubkey: content_pubkey.to_string(),
        account_dek: keys::b64(dek),
        name: name.filter(|n| !n.trim().is_empty()).unwrap_or(host_name),
        os,
        os_version,
        model,
        registered,
        relay_url,
        created_at: now_unix(),
    }
}

/// New identity, new machine, new account key. Returns the backup phrase.
pub fn create(app: &Arc<App>, device_name: Option<String>) -> anyhow::Result<Vec<String>> {
    if app.has_identity() {
        anyhow::bail!("This Device already has an identity. Remove {} first.", app.config.home.display());
    }
    let identity = Identity::generate();
    let machine = Machine::generate();
    let dek = keys::generate_dek();

    let machine_file = machine_file_for(&identity.pubkey(), &identity.content_pubkey(), &machine, &dek, false, app.relay_url(), device_name);
    *app.identity.lock().unwrap() = Some(IdentityFile::new(&identity));
    *app.machine.lock().unwrap() = Some(machine_file);
    app.save_identity()?;
    app.save_machine()?;

    // Fresh store; the sequence starts at zero for a new identity.
    {
        let mut state = app.state.lock().unwrap();
        *state = Default::default();
    }
    if let Some(device) = app.local_device() {
        upsert_device(&mut app.state.lock().unwrap().devices, device);
    }
    app.save_state();

    // The DEK wrapped to the content key, so a restore can unwrap it. Uploaded when a relay is
    // reachable; harmless to keep queued until then.
    let sealed = crate::crypto::seal(&identity.content_pubkey(), &dek)?;
    app.push_blob("key", None, sealed);
    app.push_machine_blob_if_changed();
    create_lead_bot(app);

    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(identity.phrase())
}

pub const LEAD_BOT_NAME: &str = "Chef";

/// A new account starts with one bot to talk to: a chief of staff on this Mac that proposes
/// and builds the rest of the team. It is an ordinary bot with a default profile.
fn create_lead_bot(app: &Arc<App>) {
    let Some(runner_id) = app.this_device_id() else { return };
    let bot = crate::model::Bot {
        id: String::new(),
        name: LEAD_BOT_NAME.into(),
        label: "Chief of staff".into(),
        description: "Learns what you work on, proposes a small team of one-job bots, and routes work to them.".into(),
        symbol_name: "sparkles".into(),
        accent: "indigo".into(),
        runner_id,
        provider: "deepseek".into(),
        model: None,
        instructions: "You are the user's chief of staff and the bot they talk to most. Start by learning what they work on and \
            what keeps coming back. Propose a small team of specialist bots, one clear job each, and create them with create_bot \
            once the user agrees, and refine any bot's profile with edit_bot when the user asks. In group chats, route work to the right teammate with message_bot and keep the user informed. \
            Be direct and concrete, and never invent results."
            .into(),
        workdir: None,
        created_at: 0.0,
    };
    match app.create_bot_with_dm(bot, None) {
        Ok((_, chat)) => {
            let _ = app.update_chat_meta(&chat.meta.id, |meta| meta.is_pinned = true);
            crate::runtime::prime_names(app);
        }
        Err(error) => tracing::warn!(%error, "creating the lead bot"),
    }
}

/// Restore from the backup phrase: re-derive keys, attest this machine, unwrap the DEK.
pub async fn restore(app: &Arc<App>, phrase: &str, device_name: Option<String>) -> anyhow::Result<()> {
    if app.has_identity() {
        anyhow::bail!("This Device already has an identity.");
    }
    let url = app.relay_url().ok_or_else(|| anyhow::anyhow!("Set a relay URL first. Restoring unwraps the account key from the relay."))?;
    let identity = Identity::from_master(keys::secret_from_phrase(phrase)?);
    let machine = Machine::generate();

    app.relay.register(&url, &identity, &machine.pubkey(), &machine.box_pubkey()).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let dek = crate::sync::fetch_dek(app, &url, &identity, &machine).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let machine_file = machine_file_for(&identity.pubkey(), &identity.content_pubkey(), &machine, &dek, true, Some(url), device_name);
    *app.identity.lock().unwrap() = Some(IdentityFile::new(&identity));
    *app.machine.lock().unwrap() = Some(machine_file);
    app.save_identity()?;
    app.save_machine()?;
    {
        let mut state = app.state.lock().unwrap();
        *state = Default::default();
    }
    if let Some(device) = app.local_device() {
        upsert_device(&mut app.state.lock().unwrap().devices, device);
    }
    app.save_state();
    app.push_machine_blob_if_changed();
    app.outbox_notify.notify_waiters();

    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(())
}
