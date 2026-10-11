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
    app.store.clear()?;
    if let Some(device) = app.local_device() {
        upsert_device(&mut app.state.lock().unwrap().devices, device);
    }
    app.save_state();

    // Uploaded when a relay is reachable; harmless to keep queued until then.
    push_account_key(app);
    app.push_machine_blob_if_changed();
    create_lead_bot(app);

    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(identity.phrase())
}

/// Signed ahead of the account key, so the signature passes for nothing else the identity key
/// signs.
const ACCOUNT_KEY_DOMAIN: &[u8] = b"lorca/account-key/v1\0";

/// The account key as the relay keeps it for a restore: signed by the identity and sealed to
/// the content key. Anyone can seal to the content key, the relay included; only the identity
/// signs, so a restore takes no key but the identity's.
#[derive(serde::Serialize, serde::Deserialize)]
struct AccountKey {
    dek: String,
    signature: String,
}

pub fn seal_account_key(identity: &Identity, dek: &[u8; 32]) -> anyhow::Result<Vec<u8>> {
    let signature = identity.sign(&[ACCOUNT_KEY_DOMAIN, dek].concat());
    crate::crypto::seal_json(&identity.content_pubkey(), &AccountKey { dek: keys::b64(dek), signature })
}

/// The account key in a `key` blob, when the identity sealed and signed it.
pub fn open_account_key(identity: &Identity, ciphertext: &[u8]) -> Option<[u8; 32]> {
    let key: AccountKey = crate::crypto::unseal_json(&identity.content_secret, ciphertext).ok()?;
    let dek = keys::unb64_32(&key.dek).ok()?;
    let signature = ed25519_dalek::Signature::from_slice(&keys::unb64(&key.signature).ok()?).ok()?;
    identity.signing.verifying_key().verify_strict(&[ACCOUNT_KEY_DOMAIN, &dek].concat(), &signature).ok()?;
    Some(dek)
}

/// Puts the account key on the relay, in one slot, when this Device holds the identity: after
/// creating it, for a relay that lost the account, and each time the CLI starts, so the relay
/// always holds one a restore can open.
pub fn push_account_key(app: &App) {
    let Some(identity) = app.identity.lock().unwrap().clone().and_then(|file| file.identity().ok()) else { return };
    let Some(dek) = app.dek() else { return };
    match seal_account_key(&identity, &dek) {
        Ok(sealed) => app.push_slot_blob("key", crate::app::Slot::latest("key"), None, sealed),
        Err(error) => tracing::error!(%error, "sealing the account key"),
    }
}

pub const LEAD_BOT_NAME: &str = "Chef";

/// A new account starts with one general-purpose bot to talk to. It is an ordinary bot with a
/// default profile on this Runner.
fn create_lead_bot(app: &Arc<App>) {
    let Some(runner_id) = app.this_device_id() else { return };
    let bot = crate::model::Bot {
        id: String::new(),
        name: LEAD_BOT_NAME.into(),
        description: "Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary.".into(),
        symbol_name: "sparkles".into(),
        accent: "indigo".into(),
        avatar: None,
        runner_id,
        provider: "deepseek".into(),
        model: None,
        thinking: None,
        legacy_instructions: String::new(),
        workdir: None,
        permissions: None,
            created_at: 0.0,
    };
    match app.create_bot_with_dm(bot, None) {
        Ok((_, chat)) => {
            let _ = app.update_chat_meta(&chat.meta.id, |meta| meta.is_pinned = true);
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
    app.store.clear()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Anyone can seal to the content key; a restore takes only the key the identity signed.
    #[test]
    fn a_restore_takes_only_the_account_key_the_identity_signed() {
        let identity = Identity::generate();
        let dek = keys::generate_dek();
        assert_eq!(open_account_key(&identity, &seal_account_key(&identity, &dek).unwrap()), Some(dek));

        let planted = keys::generate_dek();
        let unsigned = crate::crypto::seal(&identity.content_pubkey(), &planted).unwrap();
        assert_eq!(open_account_key(&identity, &unsigned), None);
        let other = Identity::generate();
        let signed_by_another = crate::crypto::seal_json(
            &identity.content_pubkey(),
            &AccountKey { dek: keys::b64(&planted), signature: other.sign(&[ACCOUNT_KEY_DOMAIN, &planted].concat()) },
        )
        .unwrap();
        assert_eq!(open_account_key(&identity, &signed_by_another), None);
    }
}
