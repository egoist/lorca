//! Create or restore the identity on this Device.

use std::sync::Arc;

use crate::app::{upsert_device, App};
use crate::config::now_unix;
use crate::events::Event;
use crate::keys::{self, Identity, IdentityFile, Keyring, Machine, MachineFile, SignedKeyring};
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
        keyrings: Vec::new(),
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
    publish_keyring(app);
    app.push_machine_blob_if_changed();
    create_lead_bot(app);

    app.emit(Event::IdentityChanged { has_identity: true });
    app.emit(Event::Snapshot(app.snapshot()));
    Ok(identity.phrase())
}

/// The identity's keys, on an identity device.
fn identity_keys(app: &App) -> Option<Identity> {
    app.identity.lock().unwrap().clone().and_then(|file| file.identity().ok())
}

/// The account's keyring, signed by the identity, covering every generation of the account key
/// and every unpaired Device this Device holds: an identity device signs it anew when its last
/// one does not cover them. `None` on a Device without the identity.
pub fn signed_keyring(app: &App) -> Option<SignedKeyring> {
    let identity = identity_keys(app)?;
    let signed = {
        let mut machine = app.machine.lock().unwrap();
        let file = machine.as_mut()?;
        let keyring = file.keyring().ok()?;
        if let [kept] = file.keyrings.as_slice() {
            if kept.read().is_ok_and(|kept| kept == keyring) {
                return Some(kept.clone());
            }
        }
        let signed = SignedKeyring::sign(&identity, &keyring).ok()?;
        file.keyrings = vec![signed.clone()];
        signed
    };
    if let Err(error) = app.save_machine() {
        tracing::error!(%error, "saving the account keyring");
    }
    Some(signed)
}

/// The keyring in the `key` blob a restore found, when the identity signed it.
pub fn open_restore_keyring(identity: &Identity, ciphertext: &[u8]) -> Option<(SignedKeyring, Keyring)> {
    let signed: SignedKeyring = crate::crypto::unseal_json(&identity.content_secret, ciphertext).ok()?;
    let keyring = signed.verify(&identity.pubkey()).ok()?;
    Some((signed, keyring))
}

/// What an identity device keeps on the relay for the account's keys: the keyring, sealed to
/// the content key, in the `key` slot for a restore. Anyone can seal to the content key, but a
/// restore takes only a keyring the identity signed. It goes up after creating the identity, for
/// a relay that lost the account, after a rotation, and each time the CLI starts, so the relay
/// always holds the keys a restore needs.
pub fn publish_keyring(app: &App) {
    let Some(signed) = signed_keyring(app) else { return };
    let Some(identity) = identity_keys(app) else { return };
    match crate::crypto::seal_json(&identity.content_pubkey(), &signed) {
        Ok(sealed) => app.push_slot_blob("key", crate::app::Slot::latest("key"), None, sealed),
        Err(error) => tracing::error!(%error, "sealing the account keyring"),
    }
}

/// Hands the keyrings this Device holds to every other Device it knows that was not unpaired,
/// each sealed to the Device's box key in a slot of its own (`key-<machine pubkey>`). Every
/// Device does this when its newest account key changes: the identity's signature makes a
/// keyring safe to pass on, so a Device paired from one that had not yet heard of a rotation
/// still gets it from that one.
pub fn forward_keyrings(app: &App) {
    let Some(file) = app.machine_file() else { return };
    let Ok(keyring) = file.keyring() else { return };
    if keyring.newest().generation == 0 {
        return;
    }
    let (devices, mut unpaired) = {
        let state = app.state.lock().unwrap();
        (state.devices.clone(), state.unpaired.clone())
    };
    unpaired.extend(keyring.unpaired());
    let this = app.this_device_id();
    for device in devices {
        if unpaired.contains(&device.id) || device.box_pubkey.is_empty() || this.as_deref() == Some(device.id.as_str()) {
            continue;
        }
        match crate::crypto::seal_json(&device.box_pubkey, &file.keyrings) {
            Ok(ciphertext) => app.queue_blob(crate::app::OutboxItem {
                id: uuid::Uuid::new_v4().to_string(),
                kind: "key".into(),
                recipient: Some(device.id.clone()),
                ciphertext,
                slot: Some(crate::app::Slot::latest(format!("key-{}", device.id))),
                group: None,
            }),
            Err(error) => tracing::error!(%error, "sealing the account keyring to a Device"),
        }
    }
}

/// On an identity device, makes a new generation of the account key when a Device is unpaired
/// that the newest generation does not leave out, whether the roster said so or another
/// generation made at the same time did. The new generation lists every unpaired Device, which
/// never gets it, so their copies open nothing written after.
pub fn rotate_if_due(app: &Arc<App>) {
    let Some(identity) = identity_keys(app) else { return };
    let mut unpaired = app.state.lock().unwrap().unpaired.clone();
    {
        let mut machine = app.machine.lock().unwrap();
        let Some(file) = machine.as_mut() else { return };
        let Ok(mut keyring) = file.keyring() else { return };
        unpaired.extend(keyring.unpaired());
        if unpaired.is_subset(&keyring.newest().unpaired) {
            return;
        }
        let generation = keyring.newest().generation + 1;
        keyring.keys.push(keys::GenerationKey { generation, key: keys::b64(&keys::random_32()), made_at: crate::config::now_unix(), unpaired });
        match SignedKeyring::sign(&identity, &keyring) {
            Ok(signed) => file.keyrings = vec![signed],
            Err(error) => return tracing::error!(%error, "signing the account keyring"),
        }
    }
    if let Err(error) = app.save_machine() {
        tracing::error!(%error, "saving the account keyring");
    }
    tracing::info!("made a new generation of the account key");
    crate::sync::account_key_changed(app, true, None);
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
    let (dek, keyring) = crate::sync::fetch_dek(app, &url, &identity, &machine).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut machine_file = machine_file_for(&identity.pubkey(), &identity.content_pubkey(), &machine, &dek, true, Some(url), device_name);
    machine_file.keyrings = vec![keyring];
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
    use crate::model::{Author, Body, ChatBlob, Message, RosterBlob};
    use crate::relay::BlobIn;

    struct Account {
        apps: Vec<Arc<App>>,
        homes: Vec<std::path::PathBuf>,
    }

    impl Drop for Account {
        fn drop(&mut self) {
            for home in &self.homes {
                let _ = std::fs::remove_dir_all(home);
            }
        }
    }

    /// An identity device and `others` Devices paired with it, each knowing all the others.
    fn account(others: usize) -> Account {
        let homes: Vec<_> = (0..=others).map(|_| std::env::temp_dir().join(format!("lorca-keyring-{}", uuid::Uuid::new_v4()))).collect();
        let apps: Vec<Arc<App>> = homes.iter().map(|home| App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap()).collect();
        create(&apps[0], Some("Identity".into())).unwrap();
        for app in &apps[1..] {
            let mut file = apps[0].machine_file().unwrap();
            file.machine_secret = keys::b64(&Machine::generate().secret);
            *app.machine.lock().unwrap() = Some(file);
            app.save_machine().unwrap();
        }
        let devices: Vec<_> = apps.iter().map(|app| app.local_device().unwrap()).collect();
        let state = apps[0].state.lock().unwrap().clone();
        for app in &apps {
            let mut kept = app.state.lock().unwrap();
            kept.bots = state.bots.clone();
            kept.chats = state.chats.clone();
            for device in &devices {
                upsert_device(&mut kept.devices, device.clone());
            }
        }
        Account { apps, homes }
    }

    /// Hands `app` what another Device queued, as the relay would.
    fn deliver(app: &Arc<App>, item: &crate::app::OutboxItem) {
        let blob = BlobIn { id: uuid::Uuid::new_v4().to_string(), kind: item.kind.clone(), recipient_machine_pubkey: item.recipient.clone(), seq: 1, ciphertext: keys::b64(&item.ciphertext), created_at: 0 };
        crate::sync::apply_blob(app, &app.machine_file().unwrap(), &blob);
    }

    fn text(message: &Message) -> String {
        match &message.body {
            Body::Text { text, .. } => text.clone(),
            _ => String::new(),
        }
    }

    fn latest(app: &App, kind: &str) -> crate::app::OutboxItem {
        app.store.outbox().unwrap().into_iter().rev().find(|item| item.kind == kind).unwrap()
    }

    /// Unpairing a Device makes a new generation of the account key that every other Device
    /// gets and the unpaired one does not. What the newest key encrypts opens nowhere else, and
    /// what the unpaired Device writes under the keys it kept changes nothing.
    #[test]
    fn unpairing_makes_a_key_the_unpaired_device_never_gets() {
        let account = account(2);
        let (identity, phone, laptop) = (&account.apps[0], &account.apps[1], &account.apps[2]);
        let laptop_id = laptop.this_device_id().unwrap();
        let chat = identity.state.lock().unwrap().chats[0].meta.id.clone();
        let old_keys = identity.dek().unwrap();
        let before = Message::new(&chat, Author::You, Body::text("from before"));
        let chat_item = |id: &str, keys: &keys::AccountKeys, message: &Message| crate::app::OutboxItem {
            id: id.into(), kind: "chat".into(), recipient: None, ciphertext: crate::crypto::encrypt_json(keys, "chat", &ChatBlob::Upsert { message: message.clone() }).unwrap(), slot: None, group: None,
        };
        deliver(phone, &chat_item("before", &old_keys, &before));

        identity.state.lock().unwrap().unpaired.insert(laptop_id.clone());
        rotate_if_due(identity);
        let keys = identity.dek().unwrap();
        assert_ne!(keys.newest(), old_keys.newest());
        assert_eq!(keys.each().len(), 2);
        let outbox = identity.store.outbox().unwrap();
        assert_eq!(outbox[0].kind, "key", "keyrings go up before what the new key encrypts");
        let sent: Vec<_> = outbox.iter().filter(|item| item.kind == "key").cloned().collect();
        assert!(sent.iter().any(|item| item.recipient == phone.this_device_id()));
        assert!(sent.iter().all(|item| item.recipient.as_deref() != Some(laptop_id.as_str())), "never to the unpaired laptop");
        assert!(sent.iter().any(|item| item.recipient.is_none()), "the restore copy");

        // The relay hands the phone the new roster before its keyring: it waits for the key.
        let roster = latest(identity, "roster");
        deliver(phone, &roster);
        assert!(phone.state.lock().unwrap().unpaired.is_empty());
        let mut events = phone.events.subscribe();
        for item in &sent {
            deliver(phone, item);
        }
        crate::sync::release_held(phone, &phone.machine_file().unwrap());
        assert_eq!(phone.dek().unwrap(), keys);
        assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event, crate::events::Event::AccountKeyChanged)));
        assert!(phone.state.lock().unwrap().unpaired.contains(&laptop_id), "the keyring says who was unpaired");
        assert!(phone.device(&laptop_id).is_none());
        assert!(phone.store.held_blobs(60).unwrap().is_empty(), "the roster it held was applied");

        // What the newest key encrypts does not open with the laptop's keys.
        assert!(crate::crypto::decrypt_json::<RosterBlob>(&laptop.dek().unwrap(), "roster", &roster.ciphertext).is_err());

        // What the laptop writes under its old keys changes nothing on the phone: a roster, a
        // new message, a message that is there.
        let mut planted = crate::crypto::decrypt_json::<RosterBlob>(&keys, "roster", &roster.ciphertext).unwrap();
        planted.bots[0].name = "Planted".into();
        let bots = phone.state.lock().unwrap().bots.clone();
        deliver(phone, &crate::app::OutboxItem { id: "planted".into(), kind: "roster".into(), recipient: None, ciphertext: crate::crypto::encrypt_json(&old_keys, "roster", &planted).unwrap(), slot: None, group: None });
        assert_eq!(phone.state.lock().unwrap().bots, bots);
        let new_message = Message::new(&chat, Author::You, Body::text("planted"));
        deliver(phone, &chat_item("new", &old_keys, &new_message));
        assert!(phone.message(&chat, &new_message.id).is_none());
        deliver(phone, &chat_item("rewrite", &old_keys, &Message { body: Body::text("changed"), ..before.clone() }));
        assert_eq!(phone.message(&chat, &before.id).map(|message| text(&message)), Some("from before".into()));

        assert_eq!(laptop.dek().unwrap(), old_keys, "the laptop holds the keys it had and no more");
    }

    /// Only the identity device that makes a generation puts the roster up again:
    /// another identity device's copies may be older than what the other Devices hold.
    #[test]
    fn only_the_device_that_rotates_puts_up_the_roster() {
        let account = account(1);
        let (first, second) = (&account.apps[0], &account.apps[1]);
        *second.identity.lock().unwrap() = first.identity.lock().unwrap().clone();
        let chat = second.state.lock().unwrap().chats[0].meta.id.clone();
        second.store.upsert(&Message::new(&chat, Author::You, Body::text("an old copy"))).unwrap();
        first.state.lock().unwrap().unpaired.insert(Machine::generate().pubkey());
        rotate_if_due(first);
        assert!(first.store.outbox().unwrap().iter().any(|item| item.kind == "roster"));
        let second_id = second.this_device_id().unwrap();
        let state = second.state.lock().unwrap().clone();
        for item in second.store.outbox().unwrap() {
            second.store.remove_outbox_with_state(&item.id, &state).unwrap();
        }
        for item in first.store.outbox().unwrap().iter().filter(|item| item.kind == "key" && item.recipient.as_deref() == Some(second_id.as_str())) {
            deliver(second, item);
        }
        assert_eq!(second.dek().unwrap(), first.dek().unwrap());
        let kinds: Vec<String> = second.store.outbox().unwrap().into_iter().map(|item| item.kind).collect();
        assert!(!kinds.iter().any(|kind| kind == "roster" || kind == "chat"), "{kinds:?}");
    }

    /// Two identity devices that rotate at once settle on one newest generation, and when that
    /// one does not leave out every unpaired Device, the next rotation makes one that does.
    #[test]
    fn concurrent_rotations_end_in_a_key_no_unpaired_device_has() {
        let account = account(3);
        let (first, second) = (&account.apps[0], &account.apps[1]);
        *second.identity.lock().unwrap() = first.identity.lock().unwrap().clone();
        let (gone, also_gone) = (account.apps[2].this_device_id().unwrap(), account.apps[3].this_device_id().unwrap());
        first.state.lock().unwrap().unpaired.insert(gone.clone());
        second.state.lock().unwrap().unpaired.insert(also_gone.clone());
        rotate_if_due(first);
        rotate_if_due(second);
        for (from, to) in [(first, second), (second, first)] {
            let to_id = to.this_device_id().unwrap();
            for item in from.store.outbox().unwrap().iter().filter(|item| item.kind == "key" && item.recipient.as_deref() == Some(to_id.as_str())) {
                deliver(to, item);
            }
        }
        for app in [first, second] {
            assert_eq!(app.state.lock().unwrap().unpaired, [gone.clone(), also_gone.clone()].into_iter().collect());
            assert!(app.rotation_due.load(std::sync::atomic::Ordering::Relaxed), "the other's keyring said who else was unpaired");
            rotate_if_due(app);
        }
        let newest = |app: &Arc<App>| app.machine_file().unwrap().keyring().unwrap().newest().clone();
        for app in [first, second] {
            let newest = newest(app);
            assert!(newest.unpaired.contains(&gone) && newest.unpaired.contains(&also_gone), "{newest:?}");
        }
    }

    /// A Device paired from one that had not heard of a rotation yet gets the key from that one
    /// once it has.
    #[test]
    fn a_device_passes_on_the_keyring_it_gets() {
        let account = account(2);
        let (identity, pairer, newcomer) = (&account.apps[0], &account.apps[1], &account.apps[2]);
        identity.state.lock().unwrap().unpaired.insert(Machine::generate().pubkey());
        rotate_if_due(identity);
        let pairer_id = pairer.this_device_id().unwrap();
        for item in identity.store.outbox().unwrap().iter().filter(|item| item.kind == "key" && item.recipient.as_deref() == Some(pairer_id.as_str())) {
            deliver(pairer, item);
        }
        let newcomer_id = newcomer.this_device_id().unwrap();
        let passed: Vec<_> = pairer.store.outbox().unwrap().into_iter().filter(|item| item.kind == "key" && item.recipient.as_deref() == Some(newcomer_id.as_str())).collect();
        assert_eq!(passed.len(), 1);
        deliver(newcomer, &passed[0]);
        assert_eq!(newcomer.dek().unwrap(), identity.dek().unwrap());
    }

    /// What waits in the outbox when the key changes goes up under the new key, and so does what
    /// this Device uploaded after the keyring's place in the log, unless a newer version of it
    /// waits already.
    #[test]
    fn writes_from_around_a_rotation_go_up_under_the_new_key() {
        let account = account(1);
        let (identity, phone) = (&account.apps[0], &account.apps[1]);
        let chat = phone.state.lock().unwrap().chats[0].meta.id.clone();
        let gone_up = |app: &Arc<App>, seq: i64| {
            let state = app.state.lock().unwrap().clone();
            for item in app.store.outbox().unwrap() {
                app.store.remove_outbox_with_state(&item.id, &state).unwrap();
                app.remember_upload(item, seq);
            }
        };
        let before = Message::new(&chat, Author::You, Body::text("uploaded before the keyring"));
        phone.upsert_message(before.clone(), true);
        gone_up(phone, 0);
        let after = Message::new(&chat, Author::You, Body::text("uploaded after it"));
        phone.upsert_message(after.clone(), true);
        gone_up(phone, 5);
        let edited = Message::new(&chat, Author::You, Body::text("first version"));
        phone.upsert_message(edited.clone(), true);
        gone_up(phone, 6);
        phone.upsert_message(Message { body: Body::text("newer version, waiting"), ..edited.clone() }, true);
        let waiting = Message::new(&chat, Author::You, Body::text("written offline"));
        phone.upsert_message(waiting.clone(), true);

        identity.state.lock().unwrap().unpaired.insert(keys::Machine::generate().pubkey());
        rotate_if_due(identity);
        for item in identity.store.outbox().unwrap().iter().filter(|item| item.kind == "key") {
            deliver(phone, item);
        }
        let keys = phone.dek().unwrap();
        let queued: Vec<(String, bool)> = phone
            .store
            .outbox()
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "chat")
            .map(|item| crate::sync::under_newest_key(phone, item))
            .filter_map(|item| match crate::crypto::decrypt_json::<ChatBlob>(&keys, "chat", &item.ciphertext) {
                Ok(ChatBlob::Upsert { message }) => Some((text(&message), crate::crypto::under_newest(&keys, "chat", &item.ciphertext))),
                _ => None,
            })
            .collect();
        let sent = |words: &str| queued.iter().filter(|(text, _)| text == words).map(|(_, newest)| *newest).collect::<Vec<_>>();
        assert_eq!(sent("uploaded after it"), [true]);
        assert_eq!(sent("written offline"), [true]);
        assert_eq!(sent("newer version, waiting"), [true]);
        assert!(sent("first version").is_empty(), "the newer version waiting is not replaced");
        assert!(sent("uploaded before the keyring").is_empty(), "the other Devices took that one");
    }

    /// A keyring counts only when this account's identity signed it, for this account.
    #[test]
    fn a_keyring_counts_only_from_the_accounts_identity() {
        let account = account(1);
        let phone = &account.apps[1];
        let mut file = phone.machine_file().unwrap();
        let keyring = file.keyring().unwrap();
        let mut planted = keyring.clone();
        planted.keys.push(keys::GenerationKey { generation: 1, key: keys::b64(&keys::random_32()), made_at: 1, unpaired: Default::default() });
        let another = Identity::generate();
        assert!(file.add_keyring(SignedKeyring::sign(&another, &planted).unwrap()).is_err());
        let other_account = Keyring { identity: another.pubkey(), ..planted.clone() };
        assert!(file.add_keyring(SignedKeyring::sign(&another, &other_account).unwrap()).is_err());
        let identity = account.apps[0].identity.lock().unwrap().clone().unwrap().identity().unwrap();
        let mut not_this_account = planted.clone();
        not_this_account.keys[0].key = keys::b64(&keys::random_32());
        assert!(file.add_keyring(SignedKeyring::sign(&identity, &not_this_account).unwrap()).is_err());
        assert!(file.add_keyring(SignedKeyring::sign(&identity, &planted).unwrap()).unwrap());
        assert!(!file.add_keyring(SignedKeyring::sign(&identity, &keyring).unwrap()).unwrap(), "nothing new");
        assert_eq!(file.keyrings.len(), 1);
    }

    /// Anyone can seal to the content key; a restore takes only a keyring the identity signed.
    #[test]
    fn a_restore_takes_only_a_keyring_the_identity_signed() {
        let identity = Identity::generate();
        let keyring = Keyring { identity: identity.pubkey(), keys: vec![keys::GenerationKey { generation: 0, key: keys::b64(&keys::generate_dek()), made_at: 0, unpaired: Default::default() }] };
        let signed = SignedKeyring::sign(&identity, &keyring).unwrap();
        let sealed = crate::crypto::seal_json(&identity.content_pubkey(), &signed).unwrap();
        assert_eq!(open_restore_keyring(&identity, &sealed).map(|(_, keyring)| keyring), Some(keyring.clone()));

        let unsigned = crate::crypto::seal(&identity.content_pubkey(), &keys::generate_dek()).unwrap();
        assert!(open_restore_keyring(&identity, &unsigned).is_none());
        let another = Identity::generate();
        let planted = Keyring { identity: identity.pubkey(), ..keyring };
        let signed_by_another = SignedKeyring::sign(&another, &planted).unwrap();
        let sealed = crate::crypto::seal_json(&identity.content_pubkey(), &signed_by_another).unwrap();
        assert!(open_restore_keyring(&identity, &sealed).is_none());
    }
}
