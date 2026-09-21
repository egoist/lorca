//! One suite for both backends. SQLite always runs, on a file in the temp directory. Postgres
//! runs when `LORCA_RELAY_TEST_POSTGRES` names a database, e.g.
//! `postgres://postgres:lorca@127.0.0.1:55432/lorca`. Identities are random, so runs share it.

use super::*;

async fn backends() -> Vec<(Arc<dyn Store>, Arc<Local>)> {
    let mut backends = Vec::new();
    let path = std::env::temp_dir().join(format!("lorca-relay-test-{}.db", uuid::Uuid::new_v4()));
    let local = Arc::new(Local::default());
    backends.push((open(path.to_str().unwrap(), local.clone()).await.unwrap(), local));
    if let Ok(url) = std::env::var("LORCA_RELAY_TEST_POSTGRES") {
        let local = Arc::new(Local::default());
        backends.push((open(&url, local.clone()).await.unwrap(), local));
    }
    backends
}

/// A sweep is the one write that reaches past its own identity: on the shared Postgres it
/// takes every old group mark. The sweep test writes here; a test that counts on a mark reads.
static MARKS: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

fn name(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn blob(identity: &str, id: &str, bytes: &[u8]) -> NewBlob {
    NewBlob {
        identity_pubkey: identity.into(),
        id: id.into(),
        kind: "chat".into(),
        recipient_machine_pubkey: None,
        slot: None,
        group: None,
        payload: Payload::Inline(bytes.to_vec()),
    }
}

fn slot(name: &str, keep_first: bool) -> Option<Slot> {
    Some(Slot { name: name.into(), keep_first })
}

async fn ids(store: &Arc<dyn Store>, identity: &str, machine: &str, since: i64, max_bytes: i64) -> Vec<String> {
    store.blobs_since(identity, machine, since, &[], 500, max_bytes).await.map_err(|e| format!("{e:?}")).unwrap().0.into_iter().map(|row| row.id).collect()
}

async fn used(store: &Arc<dyn Store>, identity: &str, more: i64, quota: u64) -> bool {
    // The quota check is the one window on usage the trait has.
    store.precheck_blob(identity, &name("probe"), None, more, quota).await.is_ok()
}

macro_rules! ok {
    ($e:expr) => {
        $e.await.map_err(|e| format!("{e:?}")).unwrap()
    };
}

#[tokio::test]
async fn a_slot_keeps_its_first_and_latest_blob() {
    for (store, _) in backends().await {
        let who = name("identity");
        ok!(store.insert_blob(NewBlob { slot: slot("message", true), ..blob(&who, "v1", b"a") }, 0));
        ok!(store.insert_blob(blob(&who, "other", b"other"), 0));
        ok!(store.insert_blob(NewBlob { slot: slot("message", true), ..blob(&who, "v2", b"ab") }, 0));
        ok!(store.insert_blob(NewBlob { slot: slot("message", true), ..blob(&who, "v3", b"abc") }, 0));
        assert_eq!(ids(&store, &who, "m", 0, i64::MAX).await, ["v1", "other", "v3"], "{}", store.describe());
        // 1 + 5 + 3 bytes are stored: one more fits a quota of 10, two do not.
        assert!(used(&store, &who, 1, 10).await && !used(&store, &who, 2, 10).await, "{}", store.describe());

        // A removal supersedes every version.
        ok!(store.insert_blob(NewBlob { slot: slot("message", false), ..blob(&who, "gone", b"x") }, 0));
        assert_eq!(ids(&store, &who, "m", 0, i64::MAX).await, ["other", "gone"]);
        assert!(used(&store, &who, 4, 10).await && !used(&store, &who, 5, 10).await);

        // A put of a known id answers with its seq and supersedes nothing.
        let again = ok!(store.insert_blob(NewBlob { slot: slot("message", false), ..blob(&who, "gone", b"x") }, 0));
        assert!(again.existing);
        assert_eq!(ids(&store, &who, "m", 0, i64::MAX).await, ["other", "gone"]);
    }
}

#[tokio::test]
async fn a_refused_put_leaves_the_slot_alone() {
    for (store, _) in backends().await {
        let who = name("identity");
        ok!(store.insert_blob(NewBlob { slot: slot("roster", false), ..blob(&who, "v1", b"abc") }, 4));
        assert!(store.insert_blob(NewBlob { slot: slot("roster", false), ..blob(&who, "v2", b"abcde") }, 4).await.is_err());
        assert_eq!(ids(&store, &who, "m", 0, i64::MAX).await, ["v1"], "{}", store.describe());
        assert!(used(&store, &who, 1, 4).await && !used(&store, &who, 2, 4).await);
    }
}

#[tokio::test]
async fn a_deleted_group_takes_its_blobs_and_stays_deleted() {
    let _marks = MARKS.read().await;
    for (store, _) in backends().await {
        let (who, other) = (name("identity"), name("identity"));
        let grouped = |identity: &str, id: &str, group: &str, bytes: &[u8]| NewBlob { group: Some(group.into()), ..blob(identity, id, bytes) };
        ok!(store.insert_blob(grouped(&who, "m1", "chat-a", b"hello"), 0));
        ok!(store.insert_blob(NewBlob { kind: "file".into(), payload: Payload::InFileStore { size: 100 }, ..grouped(&who, "photo", "chat-a", b"") }, 0));
        ok!(store.insert_blob(grouped(&who, "m2", "chat-b", b"stays"), 0));
        ok!(store.insert_blob(blob(&who, "loose", b"x"), 0));

        assert_eq!(ok!(store.delete_group(&who, "chat-a")), ["photo"], "{}", store.describe());
        assert_eq!(ids(&store, &who, "m", 0, i64::MAX).await, ["m2", "loose"]);
        assert!(used(&store, &who, 4, 10).await && !used(&store, &who, 5, 10).await);
        // A turn that finishes after the delete has nowhere to land.
        assert!(store.insert_blob(grouped(&who, "late", "chat-a", b"late"), 0).await.is_err());
        assert!(store.precheck_blob(&who, "late-file", Some("chat-a"), 1, 0).await.is_err());
        // Another identity's group of the same name is its own.
        ok!(store.insert_blob(grouped(&other, "m", "chat-a", b"ok"), 0));
        assert!(ok!(store.delete_group(&who, "chat-a")).is_empty());
        assert_eq!(ids(&store, &other, "m", 0, i64::MAX).await, ["m"]);
    }
}

#[tokio::test]
async fn a_page_stops_at_its_byte_budget() {
    for (store, _) in backends().await {
        let who = name("identity");
        for id in ["a", "b", "c"] {
            ok!(store.insert_blob(blob(&who, id, &[0; 40]), 0));
        }
        assert_eq!(ids(&store, &who, "m", 0, 100).await, ["a", "b"], "{}", store.describe());
        assert_eq!(ids(&store, &who, "m", 2, 100).await, ["c"]);
        // One row always goes out, however large.
        assert_eq!(ids(&store, &who, "m", 0, 10).await, ["a"]);
        let (rows, head) = ok!(store.blobs_since(&who, "m", 0, &["roster".to_string()], 500, i64::MAX));
        assert!(rows.is_empty() && head == 3, "the kind filter applies and the head is the identity's");
    }
}

#[tokio::test]
async fn machines_challenges_envelopes_and_revocation() {
    for (store, _) in backends().await {
        let (who, mac, phone) = (name("identity"), name("mac"), name("phone"));
        ok!(store.register_identity(&who, "content", &mac, "box", "attestation"));
        ok!(store.register_identity(&who, "content", &phone, "box", "attestation"));
        assert_eq!(store.register_identity(&who, "another", &mac, "box", "attestation").await.is_err(), true, "{}", store.describe());
        assert_eq!(ok!(store.machines_for(&who)).len(), 2);

        assert!(store.create_challenge("n0", &name("stranger"), now() + 60).await.is_err());
        let nonce = name("nonce");
        ok!(store.create_challenge(&nonce, &mac, now() + 60));
        assert!(store.redeem_challenge(&nonce, &phone).await.is_err(), "another machine's challenge");
        assert!(store.redeem_challenge(&nonce, &mac).await.is_err(), "a nonce is good once");
        let nonce = name("nonce");
        ok!(store.create_challenge(&nonce, &mac, now() + 60));
        assert_eq!(ok!(store.redeem_challenge(&nonce, &mac)).identity_pubkey, who);

        // An envelope is its recipient's alone, and a stranger's key takes none.
        ok!(store.insert_blob(NewBlob { recipient_machine_pubkey: Some(phone.clone()), ..blob(&who, "job", b"sealed") }, 0));
        assert!(store.insert_blob(NewBlob { recipient_machine_pubkey: Some(name("stranger")), ..blob(&who, "lost", b"sealed") }, 0).await.is_err());
        assert_eq!(ids(&store, &who, &phone, 0, i64::MAX).await, ["job"]);
        assert!(ids(&store, &who, &mac, 0, i64::MAX).await.is_empty());
        assert!(ok!(store.blob(&who, &mac, "job")).is_none());

        ok!(store.set_push_token(&who, &PushToken { machine_pubkey: phone.clone(), platform: "apns".into(), token: name("token"), environment: "production".into() }));
        assert_eq!(ok!(store.push_tokens_for(&who, &mac)).len(), 1);

        assert!(ok!(store.revoke_machine(&who, &phone)));
        assert!(!ok!(store.revoke_machine(&who, &phone)));
        assert!(ok!(store.revoked_machines()).contains(&phone));
        assert!(ok!(store.push_tokens_for(&who, &mac)).is_empty());
        assert!(used(&store, &who, 10, 10).await, "the envelope's bytes came back");
        assert!(store.register_identity(&who, "content", &phone, "box", "attestation").await.is_err(), "a revoked key stays out");
        assert!(store.create_challenge(&name("nonce"), &phone, now() + 60).await.is_err());
    }
}

#[tokio::test]
async fn the_pairing_mailbox() {
    for (store, _) in backends().await {
        let (who, nonce) = (name("identity"), name("nonce"));
        ok!(store.create_pairing(&nonce, &who, now() + 60));
        assert!(ok!(store.pair_request(&nonce, &who)).is_none());
        assert!(store.pair_request(&nonce, "someone-else").await.is_err(), "{}", store.describe());
        ok!(store.post_pair_request(&nonce, b"request"));
        assert!(store.post_pair_request(&nonce, b"second").await.is_err());
        assert_eq!(ok!(store.pair_request(&nonce, &who)).as_deref(), Some(&b"request"[..]));
        assert!(ok!(store.pair_reply(&nonce)).is_none());
        ok!(store.post_pair_reply(&nonce, &who, b"reply"));
        assert_eq!(ok!(store.pair_reply(&nonce)).as_deref(), Some(&b"reply"[..]));
        ok!(store.delete_pairing(&nonce, &who));
        assert!(store.pair_reply(&nonce).await.is_err());

        let expired = name("nonce");
        ok!(store.create_pairing(&expired, &who, now() - 1));
        assert!(store.post_pair_request(&expired, b"late").await.is_err());
        ok!(store.tick());
    }
}

#[tokio::test]
async fn presence_and_events_reach_the_sockets() {
    for (store, local) in backends().await {
        let (who, mac) = (name("identity"), name("mac"));
        let mut seat = local.hub.join(&who, &mac);
        assert!(ok!(store.socket_opened(&who, &mac, seat.id, seat.came_online)), "{}", store.describe());
        assert!(ok!(store.online(&who)).contains(&mac));

        store.publish(Event::Blobs { identity: who.clone(), recipient: None }).await;
        let signal = tokio::time::timeout(std::time::Duration::from_secs(5), seat.signals.recv()).await;
        assert_eq!(signal.ok().flatten(), Some(crate::hub::Signal::Blobs), "{}", store.describe());

        // Unpairing reaches the process that holds the socket: the key dies and the seat goes.
        store.publish(Event::Revoked { identity: who.clone(), machine: mac.clone() }).await;
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async { while seat.signals.recv().await.is_some() {} }).await;
        assert!(closed.is_ok() && local.revoked.contains(&mac), "{}", store.describe());

        let last_here = local.hub.leave(&who, seat.id);
        ok!(store.socket_closed(&who, &mac, seat.id, last_here));
        assert!(!ok!(store.online(&who)).contains(&mac));
    }
}

#[tokio::test]
async fn a_sweep_drops_stale_envelopes_and_old_group_marks() {
    let _marks = MARKS.write().await;
    for (store, _) in backends().await {
        let (who, machine) = (name("identity"), name("machine"));
        ok!(store.register_identity(&who, "content", &machine, "box", "attestation"));
        let sealed = |id: &str, kind: &str| NewBlob { kind: kind.into(), recipient_machine_pubkey: Some(machine.clone()), ..blob(&who, id, b"12345") };
        ok!(store.insert_blob(sealed("job", "job"), 0));
        ok!(store.insert_blob(sealed("answer", "response"), 0));
        ok!(store.insert_blob(blob(&who, "message", b"abc"), 0));
        ok!(store.delete_group(&who, "old-chat"));

        // Nothing is stale yet.
        assert_eq!(ok!(store.sweep(now() - 60, now() - 60)), 0, "{}", store.describe());
        assert_eq!(ids(&store, &who, &machine, 0, i64::MAX).await, ["job", "answer", "message"]);
        assert!(store.insert_blob(NewBlob { group: Some("old-chat".into()), ..blob(&who, "late", b"x") }, 0).await.is_err());

        assert_eq!(ok!(store.sweep(now() + 1, now() + 1)), 2, "{}", store.describe());
        assert_eq!(ids(&store, &who, &machine, 0, i64::MAX).await, ["message"]);
        // The ten sealed bytes went back: 3 are stored.
        assert!(used(&store, &who, 1, 4).await && !used(&store, &who, 2, 4).await, "{}", store.describe());
        ok!(store.insert_blob(NewBlob { group: Some("old-chat".into()), ..blob(&who, "late", b"x") }, 0));
    }
}

#[tokio::test]
async fn an_orphan_is_a_file_of_a_known_identity_with_no_row() {
    for (store, _) in backends().await {
        let (who, stranger) = (name("identity"), name("identity"));
        ok!(store.register_identity(&who, "content", &name("machine"), "box", "attestation"));
        ok!(store.insert_blob(NewBlob { kind: "file".into(), payload: Payload::InFileStore { size: 9 }, ..blob(&who, "kept", b"") }, 0));
        let found = [(who.clone(), "kept".to_string()), (who.clone(), "lost".to_string()), (stranger, "theirs".to_string())];
        assert_eq!(ok!(store.orphans(&found)), [(who.clone(), "lost".to_string())], "{}", store.describe());
        assert!(ok!(store.orphans(&[])).is_empty());
    }
}

#[tokio::test]
async fn a_deleted_identity_leaves_revoked_machines_and_nothing_else() {
    for (store, _) in backends().await {
        let (who, other) = (name("identity"), name("identity"));
        let (mac, phone, theirs) = (name("machine"), name("machine"), name("machine"));
        ok!(store.register_identity(&who, "content", &mac, "box", "attestation"));
        ok!(store.register_identity(&who, "content", &phone, "box", "attestation"));
        ok!(store.register_identity(&other, "content", &theirs, "box", "attestation"));
        ok!(store.insert_blob(blob(&who, "message", b"abc"), 0));
        ok!(store.insert_blob(NewBlob { kind: "file".into(), payload: Payload::InFileStore { size: 9 }, ..blob(&who, "photo", b"") }, 0));
        ok!(store.insert_blob(blob(&other, "message", b"abc"), 0));

        let deleted = ok!(store.delete_identity(&who, true));
        let mut machines = deleted.machines.clone();
        machines.sort();
        let mut expected = vec![mac.clone(), phone.clone()];
        expected.sort();
        assert_eq!((machines, deleted.files), (expected, vec!["photo".to_string()]), "{}", store.describe());

        assert!(ok!(store.machines_for(&who)).is_empty());
        assert!(ids(&store, &who, &mac, 0, i64::MAX).await.is_empty());
        assert!(used(&store, &who, 4, 4).await, "usage starts over");
        let revoked = ok!(store.revoked_machines());
        assert!(revoked.contains(&mac) && revoked.contains(&phone) && !revoked.contains(&theirs));
        // The old key stays out; the identity may come back on a new one.
        assert!(store.register_identity(&who, "content", &mac, "box", "attestation").await.is_err());
        ok!(store.register_identity(&who, "content-2", &name("machine"), "box", "attestation"));

        assert_eq!(ids(&store, &other, &theirs, 0, i64::MAX).await, ["message"]);
    }
}

#[tokio::test]
async fn stats_count_what_is_stored() {
    for (store, _) in backends().await {
        let who = name("identity");
        ok!(store.register_identity(&who, "content", &name("machine"), "box", "attestation"));
        ok!(store.insert_blob(blob(&who, "message", b"abc"), 0));
        ok!(store.insert_blob(NewBlob { kind: "file".into(), payload: Payload::InFileStore { size: 100 }, ..blob(&who, "photo", b"") }, 0));
        let stats = ok!(store.stats());
        let of = |kind: &str| stats.blobs.iter().find(|(k, _, _)| k == kind).map(|(_, count, bytes)| (*count, *bytes)).unwrap_or_default();
        if store.describe().starts_with("sqlite") {
            // This test's own file, so the totals are exact.
            let expected = Stats { identities: 1, machines: 1, active_machines: [1, 1, 1], blobs: vec![("chat".into(), 1, 3), ("file".into(), 1, 100)], usage_bytes: 103, largest_identity_bytes: 103, ..Stats::default() };
            assert_eq!(stats, expected);
        } else {
            // The other tests write to the shared database meanwhile.
            assert!(stats.identities >= 1 && stats.machines >= 1 && stats.active_machines[0] >= 1, "{stats:?}");
            assert!(of("chat").0 >= 1 && of("file") >= (1, 100) && stats.usage_bytes >= 103 && stats.largest_identity_bytes >= 103, "{stats:?}");
        }
    }
}

#[tokio::test]
async fn an_inactive_identity_is_one_nothing_touched() {
    for (store, local) in backends().await {
        let (idle, busy, online) = (name("identity"), name("identity"), name("identity"));
        let (idle_mac, online_mac) = (name("machine"), name("machine"));
        ok!(store.register_identity(&idle, "content", &idle_mac, "box", "attestation"));
        ok!(store.register_identity(&busy, "content", &name("machine"), "box", "attestation"));
        ok!(store.register_identity(&online, "content", &online_mac, "box", "attestation"));
        ok!(store.insert_blob(blob(&idle, "message", b"abc"), 0));
        let seat = local.hub.join(&online, &online_mac);
        ok!(store.socket_opened(&online, &online_mac, seat.id, true));

        // Everything here is seconds old: nobody is inactive as of a minute ago.
        let inactive = ok!(store.inactive_identities(now() - 60));
        assert!(![&idle, &busy, &online].iter().any(|who| inactive.contains(who)), "{}", store.describe());
        // As of a moment from now all three are, but for the one with a socket open.
        let inactive = ok!(store.inactive_identities(now() + 5));
        assert!(inactive.contains(&idle) && inactive.contains(&busy) && !inactive.contains(&online), "{}", store.describe());

        // Deleted without revoking: the same machine key registers again.
        let deleted = ok!(store.delete_identity(&idle, false));
        assert_eq!(deleted.machines, std::slice::from_ref(&idle_mac));
        assert!(ids(&store, &idle, &idle_mac, 0, i64::MAX).await.is_empty());
        assert!(!ok!(store.revoked_machines()).contains(&idle_mac));
        ok!(store.register_identity(&idle, "content", &idle_mac, "box", "attestation"));
        ok!(store.socket_closed(&online, &online_mac, seat.id, true));
    }
}

#[tokio::test]
async fn a_recount_leaves_honest_usage_alone() {
    for (store, _) in backends().await {
        let who = name("identity");
        ok!(store.insert_blob(blob(&who, "a", b"abc"), 0));
        ok!(store.insert_blob(blob(&who, "b", b"defgh"), 0));
        ok!(store.delete_blob(&who, "a"));
        ok!(store.recount_usage());
        // Five bytes stored, before and after.
        assert!(used(&store, &who, 1, 6).await && !used(&store, &who, 2, 6).await, "{}", store.describe());
        if store.describe().starts_with("sqlite") {
            assert_eq!(ok!(store.recount_usage()), 0);
        }
    }
}
