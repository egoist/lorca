//! What one Device addresses to another through the relay: a `job` for a Runner, its
//! `job_cancel` and `job_result`, and a `request` with its `response`. Each is signed with the
//! sending Device's machine key and sealed to the box key of the Device it is for. The relay
//! holds every box public key, so a seal alone says nothing about who sent it; the signature
//! does. The Device that opens one checks it before anything acts on it, and takes the sender
//! from the signature (docs/architecture/relay-trust.md).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::config::now_unix;
use crate::keys::{b64, unb64, MachineFile};
use crate::model::Device;

/// Signed ahead of the envelope's bytes, so a signature over one passes for nothing else the
/// machine key signs. The relay's challenge nonce, the one other thing it signs, never holds a
/// `/` or a NUL (`relay::is_challenge_nonce`).
const DOMAIN: &[u8] = b"lorca/envelope/v1\0";

/// How far ahead of this Device's clock an envelope may be stamped.
const SKEW: i64 = 60 * 60;

/// How long after it was sealed an envelope of `kind` may be opened; `None` for a kind that is
/// not one. A question's asker waits seconds for the answer. A job waits on the relay for a
/// Runner that is away, as long as the relay keeps an envelope (seven days).
pub fn max_age(kind: &str) -> Option<i64> {
    match kind {
        "request" | "response" => Some(60 * 60),
        "job" | "job_cancel" | "job_result" => Some(8 * 24 * 60 * 60),
        _ => None,
    }
}

/// What the seal holds: the envelope's bytes, and the sender's signature over exactly those.
#[derive(Serialize, Deserialize)]
struct Signed {
    envelope: String,
    signature: String,
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    /// The account's identity public key.
    identity: String,
    kind: String,
    /// The sending Device's machine public key, which verifies the signature.
    from: String,
    /// The Device it is sealed to.
    to: String,
    /// New for each envelope: a Device opens an envelope once.
    id: String,
    at: i64,
    body: T,
}

/// An envelope from a Device this one has no record of yet. The relay gives the latest version
/// of a slot a new place in the log, so a Device's `machine` blob can come after an envelope it
/// sent; sync holds the envelope and opens it again when a new Device lands.
#[derive(Debug)]
pub struct UnknownSender;

impl std::fmt::Display for UnknownSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("from a Device this one does not know")
    }
}

impl std::error::Error for UnknownSender {}

/// An envelope this Device opened: the Device whose signature it carries, and what it says.
#[derive(Debug)]
pub struct Opened<T> {
    pub from: String,
    pub body: T,
}

/// Signs `body` as this Device and seals it to `recipient`.
pub fn seal<T: Serialize>(app: &App, kind: &str, recipient: &Device, body: &T) -> anyhow::Result<Vec<u8>> {
    let machine_file = app.machine_file().ok_or_else(|| anyhow::anyhow!("This Device has no identity"))?;
    seal_as(&machine_file, kind, recipient, body)
}

fn seal_as<T: Serialize>(machine_file: &MachineFile, kind: &str, recipient: &Device, body: &T) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(max_age(kind).is_some(), "{kind} is not an envelope kind");
    anyhow::ensure!(!recipient.box_pubkey.is_empty(), "{} has not shared its key yet", recipient.name);
    let machine = machine_file.machine()?;
    let envelope = Envelope {
        identity: machine_file.identity_pubkey.clone(),
        kind: kind.to_string(),
        from: machine.pubkey(),
        to: recipient.id.clone(),
        id: uuid::Uuid::new_v4().to_string(),
        at: now_unix(),
        body,
    };
    let bytes = serde_json::to_vec(&envelope)?;
    let signature = machine.sign(&[DOMAIN, &bytes].concat());
    crate::crypto::seal_json(&recipient.box_pubkey, &Signed { envelope: b64(&bytes), signature })
}

/// Opens an envelope of `kind` sealed to this Device. It holds only when it is signed by a
/// Device of this account that this one knows from the account's encrypted records and that was
/// not unpaired, names this account, this kind, and this Device, is fresh, and was not opened
/// before; the relay can deliver it again, under any blob id, and it is refused.
pub fn open<T: DeserializeOwned>(app: &App, machine_file: &MachineFile, kind: &str, ciphertext: &[u8]) -> anyhow::Result<Opened<T>> {
    let max_age = max_age(kind).ok_or_else(|| anyhow::anyhow!("{kind} is not an envelope kind"))?;
    let machine = machine_file.machine()?;
    let signed: Signed = crate::crypto::unseal_json(&machine.box_secret, ciphertext)?;
    let bytes = unb64(&signed.envelope)?;
    let envelope: Envelope<serde_json::Value> = serde_json::from_slice(&bytes)?;
    if envelope.identity != machine_file.identity_pubkey {
        anyhow::bail!("sealed for another account");
    }
    if envelope.kind != kind {
        anyhow::bail!("sealed as {}, delivered as {kind}", envelope.kind);
    }
    if envelope.to != machine.pubkey() {
        anyhow::bail!("sealed for another Device");
    }
    let known = {
        let state = app.state.lock().unwrap();
        !state.unpaired.contains(&envelope.from) && state.devices.iter().any(|device| device.id == envelope.from)
    };
    if !known {
        return Err(UnknownSender.into());
    }
    let signature = ed25519_dalek::Signature::from_slice(&unb64(&signed.signature)?)?;
    crate::keys::verifying_key(&envelope.from)?
        .verify_strict(&[DOMAIN, &bytes].concat(), &signature)
        .map_err(|_| anyhow::anyhow!("the signature does not match its sender"))?;
    let now = now_unix();
    if envelope.at > now + SKEW {
        anyhow::bail!("stamped in the future");
    }
    if envelope.at < now - max_age {
        anyhow::bail!("too old");
    }
    if !app.store.remember_envelope(&envelope.from, &envelope.id, envelope.at + max_age + SKEW)? {
        anyhow::bail!("already opened");
    }
    Ok(Opened { from: envelope.from, body: serde_json::from_value(envelope.body)? })
}

/// Who an envelope sealed to this Device names as its sender, unchecked: for telling whether a
/// held one can be opened yet.
pub fn sender(machine_file: &MachineFile, ciphertext: &[u8]) -> Option<String> {
    let machine = machine_file.machine().ok()?;
    let signed: Signed = crate::crypto::unseal_json(&machine.box_secret, ciphertext).ok()?;
    let envelope: Envelope<serde_json::Value> = serde_json::from_slice(&unb64(&signed.envelope).ok()?).ok()?;
    Some(envelope.from)
}

/// The body of an envelope sealed to `box_secret`, unchecked: for tests that look at what a
/// Device queued.
#[cfg(test)]
pub fn body_of<T: DeserializeOwned>(box_secret: &crypto_box::SecretKey, ciphertext: &[u8]) -> T {
    let signed: Signed = crate::crypto::unseal_json(box_secret, ciphertext).unwrap();
    let envelope: Envelope<T> = serde_json::from_slice(&unb64(&signed.envelope).unwrap()).unwrap();
    envelope.body
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::app::{upsert_device, App};
    use crate::config::Config;
    use crate::keys::Machine;
    use crate::model::{Job, JobCancel, Request, Response};

    /// A Runner and a phone of one account, each knowing the other from the account's records.
    struct Pair {
        runner: Arc<App>,
        phone: Arc<App>,
        homes: Vec<std::path::PathBuf>,
    }

    impl Drop for Pair {
        fn drop(&mut self) {
            for home in &self.homes {
                let _ = std::fs::remove_dir_all(home);
            }
        }
    }

    fn pair() -> Pair {
        let homes: Vec<_> = (0..2).map(|_| std::env::temp_dir().join(format!("lorca-envelope-{}", uuid::Uuid::new_v4()))).collect();
        let runner = App::load(Config { home: homes[0].clone(), port: 0 }).unwrap();
        crate::identity::create(&runner, Some("Runner".into())).unwrap();
        let phone = App::load(Config { home: homes[1].clone(), port: 0 }).unwrap();
        let mut file = runner.machine_file().unwrap();
        file.machine_secret = b64(&Machine::generate().secret);
        file.name = "Phone".into();
        file.os = "ios".into();
        *phone.machine.lock().unwrap() = Some(file);
        phone.save_machine().unwrap();
        let devices = [runner.local_device().unwrap(), phone.local_device().unwrap()];
        for app in [&runner, &phone] {
            let mut state = app.state.lock().unwrap();
            for device in &devices {
                upsert_device(&mut state.devices, device.clone());
            }
            drop(state);
            app.save_state_now();
        }
        Pair { runner, phone, homes }
    }

    fn device(app: &App) -> Device {
        app.local_device().unwrap()
    }

    /// An envelope as anyone could write it: any fields, signed by `signer`, sealed to `to`.
    fn forge(signer: &Machine, envelope: &Envelope<serde_json::Value>, to: &Device) -> Vec<u8> {
        let bytes = serde_json::to_vec(envelope).unwrap();
        let signature = signer.sign(&[DOMAIN, &bytes].concat());
        crate::crypto::seal_json(&to.box_pubkey, &Signed { envelope: b64(&bytes), signature }).unwrap()
    }

    /// What `seal` would write from `from` to `to`, for `forge` to vary.
    fn fields(from: &App, to: &App, kind: &str, body: serde_json::Value) -> Envelope<serde_json::Value> {
        Envelope {
            identity: from.machine_file().unwrap().identity_pubkey,
            kind: kind.into(),
            from: from.this_device_id().unwrap(),
            to: to.this_device_id().unwrap(),
            id: uuid::Uuid::new_v4().to_string(),
            at: now_unix(),
            body,
        }
    }

    fn open_on<T: DeserializeOwned>(app: &App, kind: &str, ciphertext: &[u8]) -> anyhow::Result<Opened<T>> {
        open(app, &app.machine_file().unwrap(), kind, ciphertext)
    }

    fn request(verb: &str, body: serde_json::Value) -> Request {
        Request { id: format!("req-{}", uuid::Uuid::new_v4()), verb: verb.into(), body }
    }

    #[test]
    fn a_device_of_the_account_signs_what_it_seals_and_the_recipient_learns_who() {
        let pair = pair();
        let sealed = seal(&pair.phone, "request", &device(&pair.runner), &request("service.status", json!({}))).unwrap();
        let opened: Opened<Request> = open_on(&pair.runner, "request", &sealed).unwrap();
        assert_eq!(opened.from, pair.phone.this_device_id().unwrap());
        assert_eq!(opened.body.verb, "service.status");
    }

    #[test]
    fn a_seal_with_only_the_recipients_public_key_opens_nothing() {
        let pair = pair();
        let runner = device(&pair.runner);
        // What a relay can make: it holds every box public key. Unsigned, as before envelopes
        // were signed, or signed by a key of its own in the phone's name.
        let unsigned = crate::crypto::seal_json(&runner.box_pubkey, &request("mcp.save", json!({}))).unwrap();
        assert!(open_on::<Request>(&pair.runner, "request", &unsigned).is_err());
        let relay = Machine::generate();
        let in_the_phones_name = forge(&relay, &fields(&pair.phone, &pair.runner, "request", json!(request("mcp.save", json!({})))), &runner);
        assert!(open_on::<Request>(&pair.runner, "request", &in_the_phones_name).unwrap_err().to_string().contains("signature"));
        let mut as_itself = fields(&pair.phone, &pair.runner, "request", json!(request("mcp.save", json!({}))));
        as_itself.from = relay.pubkey();
        assert!(open_on::<Request>(&pair.runner, "request", &forge(&relay, &as_itself, &runner)).unwrap_err().to_string().contains("does not know"));
    }

    #[test]
    fn a_changed_or_misaddressed_envelope_is_refused() {
        let pair = pair();
        let phone_keys = pair.phone.machine_file().unwrap().machine().unwrap();
        let runner = device(&pair.runner);
        let body = json!(request("service.status", json!({})));

        // Signed bytes changed after signing.
        let envelope = fields(&pair.phone, &pair.runner, "request", body.clone());
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let signature = phone_keys.sign(&[DOMAIN, &bytes].concat());
        let mut changed = envelope;
        changed.body = json!(request("mcp.save", json!({})));
        let tampered = crate::crypto::seal_json(&runner.box_pubkey, &Signed { envelope: b64(&serde_json::to_vec(&changed).unwrap()), signature }).unwrap();
        assert!(open_on::<Request>(&pair.runner, "request", &tampered).is_err());

        let mut other_account = fields(&pair.phone, &pair.runner, "request", body.clone());
        other_account.identity = Machine::generate().pubkey();
        assert!(open_on::<Request>(&pair.runner, "request", &forge(&phone_keys, &other_account, &runner)).is_err());

        let mut other_device = fields(&pair.phone, &pair.runner, "request", body.clone());
        other_device.to = pair.phone.this_device_id().unwrap();
        assert!(open_on::<Request>(&pair.runner, "request", &forge(&phone_keys, &other_device, &runner)).is_err());

        let as_a_job = forge(&phone_keys, &fields(&pair.phone, &pair.runner, "request", body.clone()), &runner);
        assert!(open_on::<Job>(&pair.runner, "job", &as_a_job).is_err());

        let mut stale = fields(&pair.phone, &pair.runner, "request", body.clone());
        stale.at -= max_age("request").unwrap() + 1;
        assert!(open_on::<Request>(&pair.runner, "request", &forge(&phone_keys, &stale, &runner)).unwrap_err().to_string().contains("too old"));
        let mut ahead = fields(&pair.phone, &pair.runner, "request", body.clone());
        ahead.at += SKEW + 60;
        assert!(open_on::<Request>(&pair.runner, "request", &forge(&phone_keys, &ahead, &runner)).is_err());

        // A job may wait days on the relay for a Runner that was away.
        let mut waited = fields(&pair.phone, &pair.runner, "job_cancel", json!(JobCancel { job_id: "job".into() }));
        waited.at -= 3 * 24 * 60 * 60;
        assert!(open_on::<JobCancel>(&pair.runner, "job_cancel", &forge(&phone_keys, &waited, &runner)).is_ok());
    }

    #[test]
    fn an_envelope_opens_once_across_restarts() {
        let pair = pair();
        let sealed = seal(&pair.phone, "request", &device(&pair.runner), &request("service.status", json!({}))).unwrap();
        assert!(open_on::<Request>(&pair.runner, "request", &sealed).is_ok());
        assert!(open_on::<Request>(&pair.runner, "request", &sealed).unwrap_err().to_string().contains("already opened"));
        let restarted = App::load(Config { home: pair.homes[0].clone(), port: 0 }).unwrap();
        assert!(open_on::<Request>(&restarted, "request", &sealed).unwrap_err().to_string().contains("already opened"));
    }

    /// The clock jumps ahead past an envelope's expiry, the Device opens another, and the clock
    /// comes back: the first envelope does not open again.
    #[test]
    fn a_clock_that_jumps_ahead_and_back_reopens_nothing() {
        let pair = pair();
        let sealed = seal(&pair.phone, "request", &device(&pair.runner), &request("service.status", json!({}))).unwrap();
        assert!(open_on::<Request>(&pair.runner, "request", &sealed).is_ok());
        // Its record as a clock a year ahead sees it: expired.
        let year = 365 * 24 * 60 * 60;
        pair.runner.store.connection.lock().unwrap().execute("UPDATE opened_envelopes SET expires_at = expires_at - ?1", [year]).unwrap();
        assert!(pair.runner.store.remember_envelope("someone", "meanwhile", now_unix()).unwrap());
        assert!(open_on::<Request>(&pair.runner, "request", &sealed).unwrap_err().to_string().contains("already opened"));
    }

    #[test]
    fn an_unpaired_device_opens_nothing_again() {
        let pair = pair();
        let phone = pair.phone.this_device_id().unwrap();
        let sealed = seal(&pair.phone, "request", &device(&pair.runner), &request("service.status", json!({}))).unwrap();
        pair.runner.state.lock().unwrap().unpaired.insert(phone);
        assert!(open_on::<Request>(&pair.runner, "request", &sealed).is_err());
    }

    #[test]
    fn a_relays_challenge_cannot_be_an_envelope() {
        assert!(crate::relay::is_challenge_nonce(&b64(&crate::keys::random_32())));
        let envelope = [DOMAIN, b"{}"].concat();
        assert!(!crate::relay::is_challenge_nonce(&String::from_utf8(envelope).unwrap()));
        assert!(!crate::relay::is_challenge_nonce("short"));
    }

    fn deliver(app: &Arc<App>, kind: &str, ciphertext: &[u8]) {
        let blob = crate::relay::BlobIn {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.into(),
            recipient_machine_pubkey: app.this_device_id(),
            seq: 1,
            ciphertext: b64(ciphertext),
            created_at: 0,
        };
        crate::sync::apply_blob(app, &app.machine_file().unwrap(), &blob);
    }

    /// The case the report describes: a relay, holding the Runner's public key, asks it to add
    /// an MCP server and connect it. Nothing is saved and nothing starts.
    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn a_relay_cannot_add_or_start_an_mcp_server_on_a_runner() {
        let pair = pair();
        let runner = device(&pair.runner);
        let marker = pair.homes[0].join("launched");
        let config = json!({ "command": "sh", "args": ["-c", format!("touch '{}'", marker.display())] });
        let save = request("mcp.save", json!({ "name": "planted", "config": config }));
        let reconnect = request("mcp.reconnect", json!({ "name": "planted" }));
        let relay = Machine::generate();
        for forged in [
            crate::crypto::seal_json(&runner.box_pubkey, &save).unwrap(),
            forge(&relay, &fields(&pair.phone, &pair.runner, "request", json!(save)), &runner),
            crate::crypto::seal_json(&runner.box_pubkey, &reconnect).unwrap(),
            forge(&relay, &fields(&pair.phone, &pair.runner, "request", json!(reconnect)), &runner),
        ] {
            deliver(&pair.runner, "request", &forged);
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!pair.runner.config.mcp_path().exists(), "no server was saved");
        assert!(!marker.exists(), "no process started");
        assert!(pair.runner.store.outbox().unwrap().iter().all(|item| item.kind != "response"), "nothing was answered");

        // The phone asks the same, signed: the Runner saves it and answers the phone.
        let harmless = request("mcp.save", json!({ "name": "docs", "config": { "command": "true" } }));
        deliver(&pair.runner, "request", &seal(&pair.phone, "request", &runner, &harmless).unwrap());
        let answered = async {
            loop {
                if let Some(item) = pair.runner.store.outbox().unwrap().into_iter().find(|item| item.kind == "response") {
                    return item;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        };
        let answer = tokio::time::timeout(std::time::Duration::from_secs(5), answered).await.unwrap();
        assert_eq!(answer.recipient, pair.phone.this_device_id());
        let opened: Opened<Response> = open_on(&pair.phone, "response", &answer.ciphertext).unwrap();
        assert_eq!((opened.from, opened.body.error), (pair.runner.this_device_id().unwrap(), None));
        assert!(std::fs::read_to_string(pair.runner.config.mcp_path()).unwrap().contains("docs"));
        assert!(!marker.exists());
    }

    /// Compact on a Device the bot does not run on goes to the bot's Runner as a request, which
    /// compacts there and answers.
    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn compact_from_another_device_runs_on_the_bots_runner() {
        let pair = pair();
        let runner_id = pair.runner.this_device_id().unwrap();
        let bot = pair.runner.state.lock().unwrap().bots[0].clone();
        assert_eq!(bot.runner_id, runner_id);
        let chat = pair.runner.dm_with(&bot.id, None).unwrap();
        {
            let mut state = pair.phone.state.lock().unwrap();
            state.bots.push(bot);
            state.chats.push(chat.clone());
            state.device_online.insert(runner_id.clone());
        }
        pair.phone.machine.lock().unwrap().as_mut().unwrap().relay_url = Some("http://127.0.0.1:9".into());
        let phone = pair.phone.clone();
        let chat_id = chat.meta.id.clone();
        let asking = tokio::spawn(async move { crate::api::dispatch(&phone, "chats.compact", json!({ "chat_id": chat_id })).await });
        let sent = |app: &Arc<App>, kind: &'static str| {
            let app = app.clone();
            async move {
                loop {
                    if let Some(item) = app.store.outbox().unwrap().into_iter().find(|item| item.kind == kind) {
                        return item;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        };
        let question = tokio::time::timeout(std::time::Duration::from_secs(5), sent(&pair.phone, "request")).await.unwrap();
        assert_eq!(question.recipient.as_deref(), Some(runner_id.as_str()));
        deliver(&pair.runner, "request", &question.ciphertext);
        let answer = tokio::time::timeout(std::time::Duration::from_secs(5), sent(&pair.runner, "response")).await.unwrap();
        deliver(&pair.phone, "response", &answer.ciphertext);
        let answered = tokio::time::timeout(std::time::Duration::from_secs(5), asking).await.unwrap().unwrap();
        // The Runner went to compact with the bot's provider, which it has not connected.
        assert_eq!(answered, Err("DeepSeek is not connected".to_string()));
    }

    /// A Device's job carries the Device whose turn it is, for its result and the checks on it;
    /// one signed by another Device does not run.
    #[test]
    fn a_job_runs_only_for_the_device_that_signed_it() {
        let pair = pair();
        let job: Job = serde_json::from_value(json!({
            "id": "job-claimed", "chat_id": "chat", "bot_id": "bot", "kind": "turn", "trigger_message_id": "",
            "requested_by": pair.runner.this_device_id().unwrap(), "created_at": 0.0,
        }))
        .unwrap();
        deliver(&pair.runner, "job", &seal(&pair.phone, "job", &device(&pair.runner), &job).unwrap());
        assert!(pair.runner.running_jobs.lock().unwrap().is_empty());
    }

    /// The relay gives the latest version of a Device's `machine` blob a new place in the log, so
    /// an envelope from a Device that just paired can come before it. It waits for that blob.
    #[tokio::test]
    async fn an_envelope_waits_for_its_senders_record() {
        let pair = pair();
        let phone = pair.phone.this_device_id().unwrap();
        pair.runner.state.lock().unwrap().devices.retain(|device| device.id != phone);
        deliver(&pair.runner, "request", &seal(&pair.phone, "request", &device(&pair.runner), &request("nothing", json!({}))).unwrap());
        let responses = |app: &App| app.store.outbox().unwrap().into_iter().filter(|item| item.kind == "response").count();
        tokio::task::yield_now().await;
        assert_eq!(responses(&pair.runner), 0);

        let dek = pair.runner.dek().unwrap();
        let blob = crate::model::MachineBlob { device: device(&pair.phone), turns: Vec::new(), budgets: Vec::new(), watching: None };
        deliver(&pair.runner, "machine", &crate::crypto::encrypt_json(&dek, "machine", &blob).unwrap());
        crate::sync::release_held(&pair.runner, &pair.runner.machine_file().unwrap());
        for _ in 0..100 {
            if responses(&pair.runner) == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(responses(&pair.runner), 1, "answered once the phone is known");
        assert!(pair.runner.store.held_blobs(60).unwrap().is_empty());
    }

    /// A job stopped on another Device before the Runner got it does not start, in whatever order
    /// the relay delivers the two.
    #[tokio::test]
    async fn a_job_cancelled_before_it_arrives_does_not_start() {
        let pair = pair();
        let job = |id: &str| -> Job {
            serde_json::from_value(json!({
                "id": id, "chat_id": "chat", "bot_id": "bot", "kind": "turn", "trigger_message_id": "",
                "requested_by": pair.phone.this_device_id().unwrap(), "created_at": 0.0,
            }))
            .unwrap()
        };
        let runner = device(&pair.runner);
        deliver(&pair.runner, "job_cancel", &seal(&pair.phone, "job_cancel", &runner, &JobCancel { job_id: "job-stopped".into() }).unwrap());
        deliver(&pair.runner, "job", &seal(&pair.phone, "job", &runner, &job("job-stopped")).unwrap());
        assert!(pair.runner.running_jobs.lock().unwrap().is_empty());
        deliver(&pair.runner, "job", &seal(&pair.phone, "job", &runner, &job("job-wanted")).unwrap());
        assert!(pair.runner.running_jobs.lock().unwrap().contains_key("job-wanted"));
        for job in pair.runner.running_jobs.lock().unwrap().values() {
            job.cancel.cancel();
        }
    }

    /// An answer counts only from the Runner the question went to.
    #[tokio::test]
    async fn a_response_from_another_device_answers_nothing() {
        let pair = pair();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let elsewhere = Machine::generate().pubkey();
        pair.phone.pending_responses.lock().unwrap().insert("req-1".into(), (elsewhere, tx));
        let response = Response { request_id: "req-1".into(), body: json!("forged"), error: None };
        deliver(&pair.phone, "response", &seal(&pair.runner, "response", &device(&pair.phone), &response).unwrap());
        assert!(rx.try_recv().is_err());
        assert!(pair.phone.pending_responses.lock().unwrap().contains_key("req-1"));
    }
}
