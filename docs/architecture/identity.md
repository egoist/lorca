# Identity

An identity is keys you hold.

After Happy’s layering, the Device that creates or restores the identity is the identity device. It alone holds the master secret, so it alone registers the identity on a relay, again when a relay forgot the account. Every paired Device holds the account DEK, and any paired computer pairs others.

| Layer                    | What                                                                                | Where                                                                                                |
| ------------------------ | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Master secret (32 bytes) | Root. Backup as a base32 phrase (13 groups of 4).                                   | `~/.lorca/identity.json`, mode 0600. Stays on the identity device.                                 |
| Content keypair          | X25519, HKDF from master. Secret unseals the DEK. Public key seals it.              | Secret on Devices that have the master. Public key on the relay.                                     |
| Identity signing key     | Ed25519, HKDF from master.                                                          | Private local. Public key on the relay; the identity id is `hash(pubkey)`.                           |
| Account DEK              | Random XChaCha20-Poly1305 key. Encrypts roster, chats, messages, machine metadata, provider credentials. | Made locally. On the relay as a `key` blob **sealed** to the content public key; handed to each paired machine inside the sealed pairing reply. |
| Machine keypair          | One 32-byte secret per Device → HKDF → Ed25519 signing key + X25519 box key.        | `~/.lorca/machine.json`. Public keys on the relay, attested by the identity or by the Device that paired it. |
| Chat/job envelopes       | Account DEK for roster/chat/machine/credentials blobs; sealed box to the Runner’s box key for jobs. | Relay stores ciphertext.                                                                          |
| Push key                 | HKDF from the account DEK. Seals what a push says.                                  | Every Device derives it. An iPhone keeps a copy in its app group's keychain for the notification extension. |
| Ephemeral pairing key    | X25519, one handshake.                                                              | Devices; discarded after pairing.                                                                    |

AEAD envelopes are `nonce(24) || ciphertext` with the blob kind as associated data. Sealed boxes are libsodium `crypto_box_seal` (`crypto_box` crate); signatures are `ed25519-dalek`.

Recovery: restore the master secret from the backup phrase → re-derive content keys → unwrap DEKs from the relay. The backup phrase is the identity.

## Pairing a Device

1. Device A, any paired computer (the Mac and desktop apps offer Pair a Device on every one; a phone only joins), asks the relay for a pairing nonce and shows a pairing string: `lorca://pair?relay=…&id=<identity pubkey>&ek=<ephemeral pubkey>&n=<nonce>`. The CLI waits on it for ten minutes whether or not the sheet stays open (Done keeps the code good). Cancel retires it: `pair.cancel` drops the waiter and deletes the mailbox (`DELETE /v1/pair/{nonce}`), so a Device that pastes the code afterwards is told at once instead of polling out the TTL.
2. Device B pastes it (onboarding, or `lorca pair <string>`). B generates its machine keys and posts a request sealed to `ek` into the relay’s pairing mailbox (`POST /v1/pair/{nonce}/request`, no auth): its machine public key, box public key, `name`, `os`. `pair.accept` emits `pair.posted` once the request is up and then polls for the reply; `pair.abort` ends that wait, a newer `pair.accept` replaces it, and a mailbox that is gone (cancelled or expired) fails the wait with a message that says to get a fresh code.
3. A polls the mailbox, unseals the request, attests B on the relay, and posts a reply sealed to B’s box key: identity public key, content public key, the **account DEK**, and the relay URL. An identity device attests B with an identity-signed `POST /v1/identities`; any other Device sends `POST /v1/machines` with its own bearer, and the relay records that machine as B’s attester. That takes relay protocol 2, so `pair.start` on a Device without the identity key reads the relay's `/v1/health` first and fails at once on an older relay, before it shows a code.
4. B unseals the reply, saves `machine.json`, authenticates with the challenge, and uploads its `machine` blob (`name`, `os`, installed plugins and their state).
5. B syncs the roster, the account’s credentials, and the chats, and shows up in the Device list. Its sync loop starts as soon as the keys are saved, and its first pull takes everything but the messages before the chats' newest messages; `sync.account` answers once that part has landed, so onboarding learns whether the account has a provider without waiting for the chats. A Runner is ready for bots as soon as the `credentials` blob lands.

App ↔ CLI on one machine uses `127.0.0.1`; those keys are already local.

## Unpairing a Device

Any paired Device can unpair any other from its Device list (`device.unpair`), and a Device unpairs itself with `identity.forget`: Unpair on this computer's Devices pane in the Mac and desktop apps, Unpair This Phone on a phone. Either way the CLI calls `DELETE /v1/machines/{machine_pubkey}` with its bearer token: the relay drops the machine row, remembers the key in `revoked_machines`, deletes the envelopes sealed to it, closes its sync socket, and signals `machines` to the identity's other sockets. A revoked key never authenticates again: its bearer tokens are refused, its challenge answers `410 Gone`, and neither the identity nor a paired machine can attest it again. Without a bearer it attests no other machine either, so an unpaired computer pairs nobody. A Device that pairs again generates a new machine key. A Device unpairing itself gives the relay five seconds and forgets the identity either way; one the relay did not hear from stays in the other Devices' lists, offline, until one of them unpairs it.

The relay's machine list is the list of paired Devices. A Device reads it when its sync socket opens and whenever the relay signals `machines`, and drops a Device it no longer lists, so the other Devices see an unpaired one leave at once; a stale `machine` blob for a key the relay does not list is ignored. The unpaired Device learns when its socket closes and the relay refuses the next one: a `410` from the relay makes its CLI forget the identity (keys, credentials, chats), and the app shows onboarding. That holds for the identity device too: a phone can unpair a lost computer, and the backup phrase restores the identity on a new machine key.

A machine the relay lists that never sent a `machine` blob is an unknown Device (`sync::settle_unknown_machines`): the snapshot and `roster.changed` list it after the others with `unknown: true` and no name or `os`, and the apps show Unknown Device with a note to unpair a machine nobody recognizes. Any paired Device can attest one, so none stays hidden from the Device list. A Device takes a machine for unknown only once its pull has caught up since its socket opened, so one still reading the log does not mistake the Devices whose blobs it has yet to read, and only ten minutes after the relay attested it, which is time enough for a Device that pairs to send its blob (`lorca pair <string>` sends it before it exits). A blob that arrives later makes it a Device like any other.

## Devices and Runners

Every Device writes its `os` into its machine metadata blob. Values: `macos`, `linux`, `windows`, `ios`, `ipados`, `android`. The client sets it at pairing and re-sends it with presence. `name` starts as the name the machine already goes by: a Mac's Computer Name; on Linux the pretty hostname in `/etc/machine-info` (GNOME's Device Name), else the host name; `COMPUTERNAME` on Windows; a phone's device name. `--name` on `lorca pair` or `lorca identity new` gives another, and `device.rename` changes it later. `model` is the name a person knows the machine by ("MacBook Air (M5)": `system_profiler`'s machine name and chip on a Mac, where `hw.model` is an identifier such as `Mac17,3`). A CLI reads its model and OS version from the host on every run rather than from `machine.json`, and uploads its `machine` blob again when they change, or when its `version` or a self-updating CLI's `update` does ([Updates and the service](runtime.md#updates-and-the-service)).

`os` decides the Device’s role:

| `os`                        | Role       | Can                                                                                      |
| --------------------------- | ---------- | ---------------------------------------------------------------------------------------- |
| `macos`, `linux`, `windows` | **Runner** | Everything a Device can, plus be assigned bots and run Jobs with the account’s credentials. |
| `ios`, `ipados`, `android`  | Device     | Hold keys, configure account providers, read and write chats, create bots for Runners. |

Runner status is derived from `os` alone. There is no flag to opt a phone in or a desktop out. Peers read `os` from the decrypted metadata blob, so the relay never learns which Devices are Runners.

## Relay surface

The relay stores:

- Identity public key and content public key; machine signing and box public keys with their attestation: the identity’s signature, or the paired machine that attested them
- Blob ids, kinds, sequence numbers, timestamps, size
- A blob's slot: the random id of the message it is a version of, `roster`, `credentials`, `machine-<machine public key>`, or `read-<chat id>`
- A blob's group: the random id of the chat a message, read mark, or attachment belongs to, and the ids of deleted chats
- Recipient machine public key on an envelope (so a Runner can fetch its jobs)
- Which machine public keys have a sync socket open (presence), and when each last connected or disconnected
- Keys of unpaired machines, refused for good
- A phone's APNs or FCM device token, one per machine, dropped with the machine or when Apple or Google calls it dead
- Pairing mailboxes keyed by nonce, expiring after ten minutes

Nicknames, Device names and `os`, bot profiles, provider credentials, and chat text live inside encrypted blobs.

Each entity, as a Device keeps it and as the relay holds it:

| Entity             | Device                                                    | Relay                                                  |
| ------------------ | --------------------------------------------------------- | ------------------------------------------------------ |
| Identity           | Master + content + signing keys                           | Public key                                             |
| Device             | Machine keypair, `os`                                     | Machine public key + encrypted metadata blob           |
| Bot                | Decrypted profile                                         | Inside encrypted roster blobs                          |
| Routine            | Name, schedule, prompt, state                             | Inside encrypted roster blobs                          |
| Task               | Encrypted records, revisions, evidence references, and run claims | Account-encrypted task blobs; execution Jobs sealed to the assigned Runner |
| Plugin             | Manifest, variables, secrets, tokens on the Runner        | Id and state inside the Runner's encrypted machine blob |
| Channel            | Its subscription and inbox on the Runner; tokens in the plugin store | Its state inside the Runner's encrypted machine blob |
| ProviderCredential | `credentials.json` on every Device                        | Inside the encrypted `credentials` blob                |
| Secret             | A bot's, in `secrets.enc` on its Runner                   | Only as the sealed answer to its card                  |
| Chat / Message     | Account/chat DEK                                          | Encrypted blobs                                        |
| Job                | Any paired Device may create; the assigned Runner runs it | Sealed envelope to that Runner’s machine box key; deleted once run. A hard Stop sends `job_cancel` to the Device running it; the Runner seals how the turn ended (`job_result`) to the requesting Device, and lists the turn and what it is doing in its `machine` blob for every Device |

## Account credentials

The account has one set of provider credentials (`crates/cli/src/credentials.rs`, with setup in `provider_auth.rs`): API keys with optional base URLs for DeepSeek, Anthropic, OpenCode Zen, and OpenCode Go, the OAuth tokens of a ChatGPT and of a Grok sign-in, and the custom providers the user added, each with its name, protocol, base URL, key, and models (see [Providers](providers.md)). Every Device keeps the set in its core folder as `credentials.json` (mode `0600`) and the relay holds it as one `credentials` blob, encrypted with the account DEK under the slot `credentials`, so the log carries the latest version alone. A password or a key a bot asks for in a chat is not one of these: it stays on that bot's Runner ([Secret requests](secrets.md)).

- **A change** (a connect, a disconnect, a Runner refreshing OAuth tokens) stamps that provider's `changed_at`, saves the file, queues the blob, and tells the local app (`App::update_credentials`).
- **A blob that arrives** is merged provider by provider, custom ones by their kind, the later `changed_at` winning (`Credentials::merge`); a disconnect, or a deleted custom provider, is a `changed_at` with no credential beside it, so it travels like any change. A Device that holds a later change than the blob it received uploads its own set in return, so two Devices that changed different providers at once both end up with both.
- **A Device whose credentials the relay never had** (connected before the account carried them, or a relay that was reset) uploads them after its first pull has merged what the relay holds, so an older set never replaces a newer one.
- **OAuth tokens are shared.** A Runner that refreshes them publishes the new pair. One whose refresh fails because another Runner spent the refresh token first reads the set again and uses the tokens that arrived.
- **Connecting** an API key checks it against the provider from the Device where it was typed. A subscription sign-in opens the browser on that Device, and Cancel in the Mac and desktop apps' connect sheet stops one still waiting there (`providers.auth.cancel`), so finishing in the browser afterwards connects nothing; the phone uses an in-app browser so its embedded core can receive the provider's loopback callback. **Disconnecting** removes the credential from every Device, and deletes a custom provider.
- **Unpairing** a Device makes its CLI delete `credentials.json` with the rest of the account.
- **Bot create** may target any paired Runner. The relay payload is ciphertext of the profile.
