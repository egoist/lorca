# Relay trust

The relay holds every public key and every ciphertext of an account, and it decides what reaches a Device and when. A Device acts only on what it can check without the relay: what the account DEK encrypted, which only paired Devices hold, and what another Device of the account signed. This doc covers how it checks each, in `crates/cli`.

## What a Device knows of the others

A Device's record of another Device comes from that Device's `machine` blob, which the account DEK encrypts, or, on the Device that paired it, from its pairing request. The record binds the Device's machine public key, which is its id and its signing key, to its box key. The relay's machine list says which of those Devices are still paired and which are online. A machine the relay lists without a `machine` blob is an [unknown Device](identity.md#unpairing-a-device). Nothing is sealed to it, and nothing from it is opened. A Device takes a box key from a record alone, never from the machine list, so the relay cannot get an envelope sealed to a key of its own.

Unpairing a Device puts its key in the roster's `unpaired` set (`device.unpair`, `sync::unpair_device`). Every Device keeps the keys it reads there in its `unpaired_machines` table for good. When a roster leaves some out, the Device puts them back in its own roster and uploads it. It ignores a `machine` blob from an unpaired key and opens no envelope from one. A relay that lists an unpaired key again, or delivers its old `machine` blob again, does not bring the Device back. Unpairing does not change the account DEK, so an unpaired Device that kept its copy can still write what the DEK encrypts, a `machine` blob for a new key among them.

## Signed envelopes

What one Device addresses to another goes as an envelope (`envelope.rs`): a `job` for a Runner, its `job_cancel` and `job_result`, and a `request` with its `response`. The sender signs it with its machine key and seals it to the recipient's box key:

- The seal holds `{ envelope, signature }`. `envelope` is base64url JSON: `{ identity, kind, from, to, id, at, body }`, where `identity` is the account's identity public key, `from` and `to` are machine public keys, `id` is new for each envelope, and `at` is when it was sealed, in Unix seconds.
- `signature` is Ed25519 by `from` over `lorca/envelope/v1\0` followed by exactly those JSON bytes.

The recipient (`envelope::open`) unseals it and checks, in order:

1. `identity` is its account's, `kind` is the blob's kind, and `to` is itself.
2. `from` is a Device it has a record of, and not an unpaired one.
3. The signature verifies over the bytes it came with.
4. `at` is at most an hour ahead of its clock, and no older than the kind allows. A question allows an hour; the asker gives up within seconds. A job and its cancellation and result allow eight days, since a job waits on the relay for a Runner that is away as long as the relay keeps an envelope (seven days).
5. It has not opened `(from, id)` before. `opened_envelopes` in `lorca.sqlite3` keeps each pair, across restarts, until it would be too old anyway, and the newest 20,000 past that, so a clock that jumps ahead and back does not make one new again. `cancelled_jobs` keeps its newest the same way.

The body is read from the signed bytes. An envelope that fails a check does nothing, and it stays on the relay until the relay's sweep drops it after seven days. The exception is one from a Device it has no record of yet. The relay gives the latest version of a slot a new place in the log, so a Device that just paired can have its `machine` blob land after an envelope it sent. Such an envelope waits in `held_envelopes` (at most 256, for up to eight days) and is opened again when a Device this one had no record of lands. An envelope that passes is recorded before anything acts on it. It runs at most once: if the Runner stops while a request runs, it does not run it again, and the asker's wait runs out. [Handoffs](handoffs.md) and [durable tasks](tasks.md) recover interrupted turns from their own records.

The signer is who asked:

- A `request` carries no asker of its own. The Runner answers the signer, and the verbs that look at who asks (the review queue, tasks, feedback, a plugin's sign-in) get the signer.
- A `response` reaches the waiting question only when it comes from the Runner the request went to.
- A `job`'s `requested_by` must be its signer, or the job does not run. The job's result goes there. A task run checks the task's authority Runner against it. An `event` job from another Device does not run: event work comes only from the Runner's own [inbox](event-triggers.md#relay-transport).
- A `job_result` counts only from the Runner the job went to. A `job_cancel` may come from any Device of the account. The Runner keeps the job's id in `cancelled_jobs` for eight days, so the job does not start when the relay delivers it after its cancellation.

Relay protocol 4 is these envelopes. A Device on an older protocol cannot open them, and a relay started with `--min-protocol 4` tells it to update ([Protocols](protocols.md#cli--relay)).

## What the machine key signs

A machine key signs envelopes and the relay's auth challenge ([Relay](relay.md)). The relay picks the challenge, so a Device signs one only when it looks like a nonce: 16 to 128 base64url characters (`relay::is_challenge_nonce`). An envelope's signed bytes begin with `lorca/envelope/v1\0`, which no nonce does, so a relay cannot get an envelope signed by asking for a challenge answer.

## Pairing

The joining Device seals its request to the pairing string's `ek`. That key reaches it with the pairing string and never passes through the relay. The request carries a random `reply_key`. The reply (identity and content public keys, the account DEK, the relay URL) is encrypted with that key: XChaCha20-Poly1305, with `pair-reply` as associated data. A reply the relay wrote does not decrypt. The joining Device also checks the reply's identity public key against the pairing string's `id` ([Pairing a Device](identity.md#pairing-a-device)).

## The account key

The `key` blob is `{ dek, signature }` sealed to the content public key. The identity key signs `lorca/account-key/v1\0` followed by the DEK. Anyone can seal to the content key, but a restore (`sync::fetch_dek`) takes only a key the identity signed. The identity device puts it in the `key` slot when it creates the identity, when a relay lost the account, and each time its CLI starts (`identity::push_account_key`).

## What the relay decides

The relay decides what reaches a Device and when. It can hold back any blob, and it can deliver an account blob from before again, such as an older roster or a message version. It cannot write one: that takes the account DEK. Events reach a Runner signed with a subscription's secret, and the [hosted receivers](receivers.md) hold those secrets for the routines that use them. A routine that must not trust the relay takes its events from a [gateway](event-triggers.md) the user runs.
