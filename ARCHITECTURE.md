# Lorca Architecture

Source of truth for how Lorca is built. Read this before writing code.

Lorca is a Grok Bot alternative: persistent named bots, 1:1 chats, group chats, handoff, and orchestration. Work runs on **Devices you own**. The macOS app is the UI for the local Rust CLI, which it bundles and launches.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The app ships the CLI binary inside its bundle and starts `lorca serve` itself, unless one already answers on the port. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM): system materials, SF Symbols, Auto Layout, keyboard, accessibility.
3. **The CLI owns the agent loop:** inference, tools, streaming, cancellation, orchestration.
4. **The relay is zero-knowledge:** opaque blobs and public keys. Auth is a signature challenge.
5. **Every Device records its `os`.** A Device with a desktop `os` (`macos`, `linux`, `windows`) is a **Runner**. Phones and tablets (`ios`, `ipados`, `android`) are Devices, never Runners.
6. **Provider credentials live on the Runner the bot is assigned to.** You can create a bot for any paired Runner. DeepSeek and Anthropic keys and ChatGPT and Grok tokens stay on that assigned machine.
7. **A bot runs on one Runner:** that Device’s CLI.

## Three processes

```
┌─────────────────────┐     local websocket      ┌──────────────────────────┐
│  Lorca.app        │ ◄──────────────────────► │  lorca CLI (Rust)      │
│  AppKit             │     127.0.0.1           │  keys + agent loop       │
└─────────────────────┘                          └────────────┬─────────────┘
                                                              │ HTTPS
                                                              │ ciphertext + signed requests
                                                              ▼
                                                 ┌──────────────────────────┐
                                                 │  Relay (Rust, axum +     │
                                                 │  SQLite)                 │
                                                 │  public keys + blobs     │
                                                 └──────────────────────────┘
```

- **App:** native chat UI. Create or restore identity, pair, through the CLI. Launches the bundled CLI as a child process and restarts it if it exits.
- **CLI:** identity and machine keys, local websocket, encrypt/decrypt, agent loop, this Runner’s provider credentials, sync with the relay.
- **Relay:** store-and-forward API. Rust, axum, SQLite (`crates/relay`). Self-host it anywhere; clients point `LORCA_RELAY_URL` at it.

If the CLI is down, the app shows a native empty state with the launcher’s status and the manual `lorca serve` command.

## Identity

An identity is keys you hold.

Mac-first, after Happy’s layering. Until a phone exists, the first Device is the identity device.

| Layer                    | What                                                                                | Where                                                                                                |
| ------------------------ | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Master secret (32 bytes) | Root. Backup as a base32 phrase (13 groups of 4).                                   | `~/.lorca/identity.json`, mode 0600. Stays on the identity device.                                 |
| Content keypair          | X25519, HKDF from master. Secret unseals the DEK. Public key seals it.              | Secret on Devices that have the master. Public key on the relay.                                     |
| Identity signing key     | Ed25519, HKDF from master.                                                          | Private local. Public key on the relay; the identity id is `hash(pubkey)`.                           |
| Account DEK              | Random XChaCha20-Poly1305 key. Encrypts roster, chats, messages, machine metadata.  | Made locally. On the relay as a `key` blob **sealed** to the content public key; handed to each paired machine inside the sealed pairing reply. |
| Machine keypair          | One 32-byte secret per Device → HKDF → Ed25519 signing key + X25519 box key.        | `~/.lorca/machine.json`. Public keys on the relay, attested by the identity.                       |
| Chat/job envelopes       | Account DEK for roster/chat/machine blobs; sealed box to the Runner’s box key for jobs. | Relay stores ciphertext.                                                                          |
| Push key                 | HKDF from the account DEK. Seals what a push says.                                  | Every Device derives it. An iPhone keeps a copy in its app group's keychain for the notification extension. |
| Ephemeral pairing key    | X25519, one handshake.                                                              | Devices; discarded after pairing.                                                                    |

AEAD envelopes are `nonce(24) || ciphertext` with the blob kind as associated data. Sealed boxes are libsodium `crypto_box_seal` (`crypto_box` crate); signatures are `ed25519-dalek`.

Recovery: restore the master secret from the backup phrase → re-derive content keys → unwrap DEKs from the relay. The backup phrase is the identity.

### Pairing a Device

1. Device A (has the identity) asks the relay for a pairing nonce and shows a pairing string: `lorca://pair?relay=…&id=<identity pubkey>&ek=<ephemeral pubkey>&n=<nonce>`. The CLI waits on it for ten minutes whether or not the sheet stays open (Done keeps the code good). Cancel retires it: `pair.cancel` drops the waiter and deletes the mailbox (`DELETE /v1/pair/{nonce}`), so a Device that pastes the code afterwards is told at once instead of polling out the TTL.
2. Device B pastes it (onboarding, or `lorca pair <string>`). B generates its machine keys and posts a request sealed to `ek` into the relay’s pairing mailbox (`POST /v1/pair/{nonce}/request`, no auth): its machine public key, box public key, `name`, `os`. `pair.accept` emits `pair.posted` once the request is up and then polls for the reply; `pair.abort` ends that wait, a newer `pair.accept` replaces it, and a mailbox that is gone (cancelled or expired) fails the wait with a message that says to get a fresh code.
3. A polls the mailbox, unseals the request, attests B on the relay with an identity-signed `POST /v1/identities`, and posts a reply sealed to B’s box key: identity public key, content public key, the **account DEK**, and the relay URL. Provider credentials stay on each Runner.
4. B unseals the reply, saves `machine.json`, authenticates with the challenge, and uploads its `machine` blob (`name`, `os`, connected provider kinds, installed plugins and their state).
5. B syncs the roster and chats and shows up in the Device list. If B is a Runner, connecting a provider on B happens on B.

App ↔ CLI on one machine uses `127.0.0.1`; those keys are already local.

### Unpairing a Device

Any paired Device can unpair any other from its Device list (`device.unpair`), and a Device unpairs itself with `identity.forget`. Either way the CLI calls `DELETE /v1/machines/{machine_pubkey}` with its bearer token: the relay drops the machine row, remembers the key in `revoked_machines`, deletes the envelopes sealed to it, and wakes the identity's long-polls. A revoked key never authenticates again: its bearer tokens are refused, its challenge answers `410 Gone`, and the identity cannot re-attest it. A Device that pairs again generates a new machine key.

The relay's machine list is the list of paired Devices. Every sync cycle refreshes presence from it and drops a Device it no longer lists, so the other Devices see an unpaired one leave within a poll; a stale `machine` blob for a key the relay does not list is ignored. The unpaired Device learns on its next relay call: a `410` from the relay makes its CLI forget the identity (keys, credentials, chats), and the app shows onboarding. That holds for the identity device too: a phone can unpair a lost Mac, and the backup phrase restores the identity on a new machine key.

### Devices and Runners

Every Device writes its `os` into its machine metadata blob. Values: `macos`, `linux`, `windows`, `ios`, `ipados`, `android`. The client sets it at pairing and re-sends it with presence. `model` is the name a person knows the machine by ("MacBook Air (M5)": `system_profiler`'s machine name and chip on a Mac, where `hw.model` is an identifier such as `Mac17,3`). A CLI reads its model and OS version from the host on every run rather than from `machine.json`, and uploads its `machine` blob again when they change.

`os` decides the Device’s role:

| `os`                        | Role       | Can                                                                                      |
| --------------------------- | ---------- | ---------------------------------------------------------------------------------------- |
| `macos`, `linux`, `windows` | **Runner** | Everything a Device can, plus hold provider credentials, be assigned bots, and run Jobs. |
| `ios`, `ipados`, `android`  | Device     | Hold keys, read and write chats, create bots for Runners, pair other Devices.            |

Runner status is derived from `os` alone. There is no flag to opt a phone in or a desktop out. Peers read `os` from the decrypted metadata blob, so the relay never learns which Devices are Runners.

### Relay surface

The relay stores:

- Identity public key and content public key; machine signing and box public keys with the identity’s attestation
- Blob ids, kinds, sequence numbers, timestamps, size
- Recipient machine public key on an envelope (so a Runner can fetch its jobs)
- Last-seen of a machine public key (presence: online within 150 s)
- Keys of unpaired machines, refused for good
- A phone's APNs or FCM device token, one per machine, dropped with the machine or when Apple or Google calls it dead
- Pairing mailboxes keyed by nonce, expiring after ten minutes

Nicknames, Device names and `os`, bot profiles, and chat text live inside encrypted blobs.

## Domain model

Plaintext lives **on Devices**:

```
Identity 1──* Device
Device   1──* Bot          (only a Runner: os is macos, linux, or windows)
Identity 1──* Chat
Chat     *──* Bot          (kind dm: exactly 1 bot, fixed · kind group: 1–6 bots, members change)
Chat     1──* Message
Bot      1──* Routine      (a scheduled task, run in the bot's DM on its Runner)
Device   1──* Plugin       (an MCP server installed on a Runner, for every bot there)
Bot      1──* Job          (a turn on the bot's Runner)
```

| Entity             | Device                                                    | Relay                                                  |
| ------------------ | --------------------------------------------------------- | ------------------------------------------------------ |
| Identity           | Master + content + signing keys                           | Public key                                             |
| Device             | Machine keypair, `os`, local provider creds (Runner only) | Machine public key + encrypted metadata blob           |
| Bot                | Decrypted profile                                         | Inside encrypted roster blobs                          |
| Routine            | Name, schedule, prompt, state                             | Inside encrypted roster blobs                          |
| Plugin             | Manifest, variables, secrets, tokens on the Runner        | Id and state inside the Runner's encrypted machine blob |
| ProviderCredential | Assigned Runner’s keychain                                | —                                                      |
| Chat / Message     | Account/chat DEK                                          | Encrypted blobs                                        |
| Job                | Any paired Device may create; the assigned Runner runs it | Sealed envelope to that Runner’s machine box key; deleted once run. A `room_turn` answers with a `job_result` sealed to the requesting Device |

A bot's look is an SF Symbol (`symbol_name`) on an accent gradient (`accent`), or a profile image of the user's own: `avatar` is an attachment record whose bytes travel as an encrypted `file` blob, the same way a message attachment does, and every Device shows the image in place of the symbol once it has fetched it. Clicking a bot's avatar in the Mac inspector opens the Look sheet; on the phone, tapping the avatar in Details slides the Look screen in inside the same form sheet (`app/chat-info` has its own stack); `bots.update` takes `symbol_name`, `accent`, and `avatar` (a `{ path, … }` file to store and upload, or `null` to remove).

Creating a bot for Runner B from Device A: A writes an encrypted bot profile into the roster (paired Devices can read it) and pins B’s machine id. Bot create rejects a target whose `os` is not desktop. Turns are job envelopes addressed to B. B decrypts the job, runs the loop with B’s provider credentials, and uploads encrypted replies.

If B is offline or still connecting a provider, the envelope waits on the relay until B fetches it. The UI infers that from decrypted roster state.

## Website

`web/` is the app's site: TanStack Start (React, file routes under `web/src/routes`), Tailwind and shadcn/ui components, built with Vite and served by a Cloudflare Worker (`web/wrangler.jsonc`, `@cloudflare/vite-plugin`, static assets alongside the SSR entry). One page: hero, screenshots of the app in `LORCA_MOCK=1` mode (`web/public/screens`), features, how it works, privacy, FAQ. `bun run web` serves it locally on port 3000; `bun run web:deploy` builds and runs `wrangler deploy`.

## Phone app

`mobile/` is an Expo app (React Native, expo-router, TypeScript) for iOS and Android. A phone is a Device and never a Runner, and it links the same Device core the CLI is built from rather than porting it: `crates/mobile` (`lorca-mobile`) wraps the `lorca` library, built without the `runner` and `server` features, in a UniFFI object with three calls, and `mobile/modules/lorca-core` is the Expo native module around it.

- **The core** (`crates/mobile/src/lib.rs`): `Core.start(home, name, os, osVersion, model, listener)` loads the App from the app's own folder, sets the host facts (a phone cannot probe them), and starts the relay sync loop on an embedded tokio runtime; `request(method, params)` is one call of the JSON API in `crates/cli/src/api.rs`, blocking on the module's background queue, answering `{ result }` or `{ error: { message } }`; `wake()` asks the relay again now (the app calls it on foreground, since iOS killed the poll in flight). Every event on the App's bus reaches the listener as the `{ event, data }` frame the websocket carries.
- **The native module** (`mobile/modules/lorca-core`, picked up by Expo autolinking): a Swift `Module` and a Kotlin one, each about forty lines, forwarding `start`, `request`, and `wake` and turning the listener's frames into a `sendEvent`. `bun run core` (`build.ts`) builds the Rust for `aarch64-apple-ios` and the simulator into an xcframework with the UniFFI Swift beside it, and for `arm64-v8a` and `x86_64` with `cargo ndk` into `jniLibs` with the Kotlin (over JNA). The built pieces are gitignored; a Rust change needs `bun run core` and a native rebuild, a JS change is a Metro reload.
- **The JS side** (`mobile/src/core`): `engine.ts` starts the core, mirrors its events into the zustand store (`store.ts`, the way the Mac app's AppStore mirrors the CLI: `snapshot`, `roster.changed`, `message.*`, `job.*`, `chat.usage`, `relay.status`, `identity.changed`), and offers the screens the same verbs the Mac app has, each one request (`chats.send`, `bots.create`, `pair.accept`, `device.rename`, `device.unpair`, `identity.forget`, `sync.wake`, …). `prefs.ts` keeps the one thing that is the phone's own, the dictation language. Nothing about the account is stored outside the core's folder.
- **Pairing**: the phone scans the QR code the Mac shows, pastes the string, or opens it as a `lorca://pair?…` link; `pair.accept` does the handshake in the core and the first `bootstrap` snapshot fills the store.
- **Attachments**: the composer's `+` offers the photo library, the camera, and the file picker; a picked file's path goes with `chats.send`, and the core copies, encrypts, and uploads it as a `file` blob ahead of the message. A bubble that shows an attachment this phone does not have asks `files.path`, which fetches the blob and answers with the file. Images render as thumbnails sized from the width and height in the message (full screen on tap); other files as a name-and-size card.
- **Dictation**: the primary disc is Dictate while the field is empty (a small microphone sits inside the field once there is text). While recording, the field shows Grok Bot's pill: a stop square, the elapsed time, and bars that follow the microphone, with Send still beside it. The words land in the field after whatever was typed when the user taps the square, or go out at once when the user taps Send. The recognizer (`expo-speech-recognition`) listens in the first of the phone's preferred languages it supports (`src/ui/dictation.ts`; a Chinese speaker on an English-region phone gets zh-CN), or the language chosen in Settings or by a long press on the microphone.
- **Notifications** (`src/core/push.ts`, `expo-notifications`): after pairing, and on every launch and foreground, the app asks for permission once, reads the native device token, and registers it with the relay through the core (`push.register { platform, token, environment }`). The Swift module writes the core's push key into the keychain of the app group `group.app.lorca` at start and on `identity.changed` (removed when the phone holds no account); the notification service extension is its own target, generated at prebuild by `@bacons/apple-targets` from `targets/notify`. A tap opens the chat, a reply in the chat on screen makes no banner, and opening a chat dismisses its notifications.
- **Turns**: sending is `chats.send`; the core seals a `turn` Job to the bot's Runner or runs a group's room exchange itself, exactly as the CLI does on a Mac, and the `job.started` / `job.finished` events drive the "is working" row and "Chef stopped without replying". There is no Stop control, as in Grok Bot: a turn runs to its end.

The UI is native: a native stack with large titles, search, and toolbar items; a composer after Grok Bot's phone app, a liquid-glass `+` button and glass pill (`expo-glass-effect`, a filled pill where glass is unavailable) floating over the transcript with the Dictate or Send disc inside the pill's right edge, riding the keyboard as a `KeyboardStickyView` while the transcript's `KeyboardChatScrollView` lifts the last messages with the keys by growing its bottom inset, so the list is never resized; form sheets for chat info, new bot, new group, and settings; Link previews and context menus on chat rows; SF Symbols (Material Symbols on Android); system colors; haptics. The transcript follows the Mac app: bubbles with the bot's name above and its avatar beside the bubble's bottom edge in a group, neither in a DM; "Today 4:13 AM" separators after fifteen minutes; the "is working" row and the breathing green dot on avatars; "Message from ◉ X" and "Messaged ◉ X" markers; tool calls never shown; sidebar-style previews and stamps.

## Credential locality

- **Provider setup** on that Runner (keychain or `~/.lorca/credentials`, mode `0600`).
- **Bot create** may target any paired Runner. The relay payload is ciphertext of the profile.

## Relay

`crates/relay`: Rust, axum, rusqlite (bundled SQLite). `lorca-relay --bind 127.0.0.1:8787 --db lorca-relay.db`; `LORCA_RELAY_SECRET` signs bearer tokens (random per boot when unset); `--quota-bytes` (`LORCA_RELAY_QUOTA_BYTES`) caps stored ciphertext per identity, 0 for none.

Storage is one WAL database (`synchronous = NORMAL`) behind a single writer connection and a pool of reader connections, one per core; every query runs on tokio's blocking pool. All SQL lives in `db.rs`; `routes.rs` only decides what to ask for. A blob write wakes the long-polls of its identity alone (a `Notify` per identity, armed before each query so nothing is missed between the query and the wait). `last_seen` is written at most every 30 s per machine. The hosted relay is one process; a second instance would need a shared store and a shared wakeup, which is when a Postgres backend replaces this one.

`file` ciphertext never enters the database (`store.rs`): it goes to a directory (`--files-dir`, default `lorca-relay.files` beside the database) or, with `--s3-bucket` and `--s3-endpoint` (access keys from `LORCA_RELAY_S3_*` or `AWS_*`), to an S3-compatible bucket (AWS, R2, MinIO) over SigV4 with path-style URLs. Objects are keyed `<identity pubkey>/<blob id>`; the row keeps the metadata with an empty `ciphertext`. The object goes up before the row and is removed when the row is refused or deleted.

Rate limits (`limit.rs`) are token buckets in memory: per client IP on the routes that need no token (registration, auth, the pairing mailbox; `--ip-per-minute`, default 60, `--trust-proxy` to read `X-Forwarded-For`), and per identity on everything behind a bearer (`--identity-per-second`, default 50, burst ten times that). Over the limit is 429 with `Retry-After`.

Auth is per machine:

1. `POST /v1/auth/challenge { machine_pubkey }` → nonce.
2. Client signs the nonce with the machine signing key: `POST /v1/auth/verify`.
3. Relay issues a one-hour HMAC bearer bound to the identity and machine.

Identity-level writes are signed requests: `{ payload: base64url(json), signature }` where the payload carries `identity_pubkey` and `ts` and the identity key signed the bytes. `POST /v1/identities` registers the identity (idempotent) and attests one machine; it is used at create, restore, and for every pairing.

Tables:

- `identities(pubkey, content_pubkey, created_at)`
- `machines(machine_pubkey, identity_pubkey, box_pubkey, attestation, last_seen, created_at)`
- `blobs(identity_pubkey, id, kind, recipient_machine_pubkey nullable, seq, ciphertext, size, created_at)`, keyed on `(identity_pubkey, id)`
- `sequences(identity_pubkey, seq)`, `usage(identity_pubkey, bytes)`, `challenges`, `pairings(nonce, identity_pubkey, request, reply, expires_at)`

- `push_tokens(machine_pubkey, identity_pubkey, platform, token, environment, updated_at)`

`kind` is `roster` | `chat` | `job` | `job_result` | `machine` | `key` | `file` | `request` | `response`. Ciphertext is bytes; the nonce sits inside it. `seq` increases per identity. A Device’s `name` and `os` are inside its `machine` blob, not columns. A `file` blob is an attachment's bytes under the attachment's id, up to 24 MB of ciphertext (other kinds 4 MB); Devices poll with an explicit kinds list that leaves `file` out and fetch one by id when a transcript needs it.

Blob API: `PUT /v1/blobs` (client-chosen id of up to 64 characters in `[A-Za-z0-9._-]`, idempotent; 413 over quota), `GET /v1/blobs?since=<seq>&kinds=&wait=25` (long-poll; returns blobs for the identity that are unaddressed or addressed to the caller’s machine, filtered by kind in the query), `GET /v1/blobs/{id}` (one blob, same visibility), `DELETE /v1/blobs/{id}`, `GET /v1/machines` (presence).

Push API (`push.rs`): a phone registers where its pushes go with `PUT /v1/push/token { platform: apns | fcm, token, environment? }` (`sandbox` for a development build's APNs token; `DELETE` removes it). Any Device asks for a push with `POST /v1/push { ciphertext }`, at most 2560 bytes; the relay answers `{ queued }` at once and then sends to every token of the identity but the caller's. To APNs that is an alert with fixed words ("Lorca", "New reply"), `mutable-content`, and the ciphertext as `c`, over HTTP/2 with an ES256 provider token from the team's `.p8` key (`--apns-key`, `--apns-key-id`, `--apns-team-id`, `--apns-topic`, default `app.lorca`; the token's environment picks Apple's sandbox or production host). To FCM it is a data-only message carrying `c`, with an access token minted from a service account (`--fcm-service-account`). A relay with neither key queues nothing. `LORCA_RELAY_APNS_URL` and `LORCA_RELAY_FCM_URL` point either at a test server.

Clients set `LORCA_RELAY_URL` or the relay URL in Settings › Advanced. Without a relay the CLI works on one Device alone. In dev (`bun run dev` sets `LORCA_DEV=1` and runs a relay on `0.0.0.0:8787`), a Device with no relay configured defaults to `http://<this Mac's LAN IP>:8787`, so Pair a Device shows a code a phone on the same network can use.

A CLI lists blobs for its identity and envelopes for its machine public key (long-poll), decrypts, and emits events to the app on localhost. A page of twenty or more blobs is a backlog (a fresh pair replays the whole history): the CLI applies it with message and roster events held back and no state write per message, saves once, and emits one `snapshot` at the end of the page, so the app fills in at once instead of one message at a time.

## CLI (runtime)

`crates/cli`, binary `lorca`. Local websocket for the app. Files under `~/.lorca/` (override with `LORCA_HOME`), all mode 0600: `identity.json` (master secret, identity devices only), `machine.json` (machine secret, identity and content public keys, account DEK, `name`, `os`), `credentials.json` (this Runner’s providers), `settings.json` (relay URL), `state.json` (plaintext roster, chats, sync cursor, outbox), `files/<attachment id>` (attachment bytes, sent from here or fetched from the relay).

Commands:

- `lorca serve` — default; the app connects here
- `lorca identity new` / `identity restore <phrase>` / `identity show`
- `lorca pair` — show a pairing string and wait; `lorca pair <string>` joins
- `lorca status` / `doctor`

Bind: `127.0.0.1:4862` (`--port`, `LORCA_PORT`). Relay: `LORCA_RELAY_URL`. If the port is busy the CLI fails loudly.

### Agent loop

`crates/agent` (`lorca-agent`) is a port of pi-agent-core: `run_agent_loop` / `run_agent_loop_continue`, the same event sequence (`agent_start`, `turn_start`, `message_start/update/end`, `tool_execution_start/update/end`, `turn_end`, `agent_end`), steering and follow-up queues, `before_tool_call` / `after_tool_call` / `should_stop_after_turn` hooks, sequential or parallel tool execution, and `terminate` hints from tools, plus pi's harness behaviors: a model catalog with rates and windows, thinking levels, cost on every message, compaction, and turn-level retry. Tool arguments are salvaged from cut-off JSON, coerced and checked against the tool's schema before a call runs (a failure is an error result the model reads), and never run from a message the token limit cut off; a `prepare_next_turn` hook can swap the context or provider between turns. Every adapter puts the transcript through the same transform first (another model's thinking as text, failed turns left out, a result for every call, images downgraded for a text-only model) and retries a request that fails before it streams. A `Provider` turns a request into a stream of assistant events and never fails: errors become an assistant message with `stop_reason` `error` or `aborted`. The crate also ships `agent::harness::AgentHarness`, a general agent over the loop for hosts other than Lorca (model switching through a provider factory, skills and prompt templates, queues, hooks, events, an example chat), with persistence left to the host; the Lorca CLI uses the loop directly. `docs/agent/` is the crate's own documentation.

```
on decrypted Job:
  build context from the chat (this bot's turns are assistant, other bots' text is user "[Name]: …",
                               this bot's tool rows become tool_call + tool_result pairs)
  run_agent_loop_continue(context, provider = this Runner's creds, tools, sink)
    events → job.started / job.finished / job.retry (chat, bot) → local app WS, which shows the bot at work
           → each finished turn's usage → chat.usage (context size, cost so far)
           → a reply grows in chunks, not tokens: the text so far is re-sent at paragraph ends,
             or at a sentence end after 1.5 s of silence, never mid-word; the end sends it complete
           → every chunk and every completed message is encrypted with the account DEK → relay,
             so a paired phone watches the reply grow the way the local app does
    tool calls execute
  a turn that ended with something said → a push to the identity's phones (see Notifications)
```

Turns in one chat run one at a time on a Runner. `chats.stop` cancels the running turn (and the room exchange it belongs to).

### Notifications

A finished reply reaches the user wherever they are not looking.

- **Phones** (`crates/cli/src/push.rs`): when a turn ends with something said (not a pass, not a failure), the Runner seals a notice `{ title: the bot's name, subtitle: the group's title, body: the first 280 characters, chat_id }` and posts it to the relay's `/v1/push`. The envelope is `nonce(12) || ciphertext` of ChaCha20-Poly1305 under the push key with `push` as associated data: the IETF cipher, so iOS opens it with CryptoKit alone. Apple, Google, and the relay see "New reply" and ciphertext. On an iPhone the notification service extension (`mobile/targets/notify`) reads the push key from the app group's keychain, opens `c`, and sets the title, subtitle, body, thread, and `chat_id` before the alert shows; on Android `PushService` in the native module opens it through the core (`push_open(home, c)`, which needs only `machine.json`) and posts the notification. A push that cannot be opened shows the fixed words.
- **The Mac app** (`Notifier.swift`) posts a system notification itself from the CLI's events: on `job.finished` it takes what the bot said last in that turn, for a turn on this Runner or another. A click brings the app forward on the chat; opening a chat clears what was posted for it.
- **Watching**: neither fires for a reply the user watches arrive. The Mac app tells its CLI which chat is on screen while it is frontmost (`ui.watching { chat_id | null }`, cleared when the app disconnects); the Runner skips the push for that chat and the app skips its own notification. The phone app shows no banner for the chat it has open.

### Tools

Team tools (the CLI):

- `message_bot { bot, message }` — a message to a bot outside the current chat. The caller's chat shows a "Messaged ◉ X" marker; the target's own DM gets a "Message from ◉ X" marker and a `message` Job (with `from_bot_id` and `hops`) whose envelope goes to the **target bot’s Runner**. The tool refuses a member of the same group: they read that chat and take their own turn.
- `list_teammates` — decrypted local roster with Runner and online state.
- `memory_update { action: append | replace | remove | supersede, text?, old_text? }` — changes the bot’s curated `MEMORY.md` one fact at a time (see Memory below).
- `memory_log { text }` — one line in the bot’s daily log.
- `recall { query?, since?, until?, limit? }` — searches the bot’s memory files and every chat it is in, by words and by time (`24h`, `3d`, `today`, `yesterday`, a date).
- `create_bot { name, label, description?, instructions, provider?, workdir? }` — a new teammate on the caller’s Runner with its own DM; in a group chat it joins that chat. This is how a lead bot builds its team.
- `edit_bot { bot, name?, label?, description?, instructions?, provider?, workdir? }` — changes a teammate’s profile, or the caller’s own. Only the passed fields change and instructions replace in full; the new profile applies from that bot’s next turn.
- `routines { action: list | create | edit | pause | resume | run | delete, routine?, name?, schedule?, prompt?, enabled? }` — the caller’s own routines (see Routines below).
- `search_plugins { query? }` / `install_plugin { plugin }` / `connect_plugin { plugin }` — the marketplace, an install on the caller’s Runner that the user allows on a permission card first, and a sign-in card for an installed plugin (see Plugins below).

Plugin tools (`<plugin>__<tool>`, such as `github__create_issue`) come from the MCP servers of the plugins installed on the bot's Runner.

Coding tools (`lorca_agent::tools`, ports of pi’s built-ins, same schemas and truncation rules: 2000 lines / 50KB, whichever first):

- `read { path, offset?, limit? }`, `write { path, content }`, `edit { path, edits: [{ oldText, newText }] }`
- `bash { command, timeout? }` — tail-truncated, full output to a temp file, process group killed on stop
- `grep { pattern, path?, glob?, ignoreCase?, literal?, context?, limit? }`, `find { pattern, path?, limit? }`, `ls { path?, limit? }` — respect `.gitignore` through the `ignore` walker, no `rg`/`fd` needed

Every bot has a working directory on its Runner (`workdir`, default `~/.lorca/workspaces/<bot id>`, so a rename never moves files); relative paths resolve there.

### The lead bot

A new identity starts with one bot, **Chef**, a chief of staff on the first Mac: an ordinary bot with a default profile whose instructions are to learn the user’s work, propose a small team of one-job bots, create them with `create_bot`, and route work with `message_bot`. Nothing about it is privileged; rename or delete it like any bot.

A chat has a `kind`. A DM is one bot and never gains or loses members; there is one DM per bot. A group holds one to six bots, can add or remove them after creation, and has an `owner_bot_id` (the bot holding the work; defaults to the first member, `chats.set_owner` changes it).

Who answers, after Grok Bot's rooms:

- **DM:** its bot, always. An `@Name` is a reference the bot acts on: it calls `message_bot`, which delivers the message into that bot's own DM with the user, where that bot answers (and can message back).
- **Attachments:** a user message can carry files. Their bytes travel as `file` blobs; before a turn the Runner fetches any it lacks (`files::prefetch`), copies each into the bot's workspace at `attachments/<attachment id>/<name>` (a stable path, so every turn names the same file), and the transcript's user message gets a line per file naming that path, plus the pixels as an image content part for an image up to 5 MB, so a vision-capable model sees it and any model can open it with its tools.
- **Group:** a room exchange (`run_room`). The Device that received the user's message offers every member a turn, one at a time, in chat order with the members the message names by `@` first. A member's turn is a `room_turn` Job with the whole transcript plus an ephemeral cue (round number, how many messages are new to it); the member replies to the group or answers `PASS`, which never becomes a bubble. A round with at least one reply is followed by another, offered only to members who have heard something new; the exchange ends after a silent round or after the fourth round, which is marked winding down so members add only what is essential. The user's next message in that chat waits for the exchange (the chat lock); `chats.stop` cancels it. A member on another Runner gets its job through the relay and reports back with a `job_result` blob (`sent`, `pass`, `error`) sealed to the requesting Device; the room waits up to five minutes for it and skips an offline Runner.
- **Bot to bot:** `message_bot` from any chat to a bot outside it; the message lands in the target's DM. Each hop carries `hops`; after eight bot-to-bot hops without a user message the tool refuses, so two bots cannot loop. Members of one group talk to each other in the group.

### Routines

A routine is a task a bot runs on a schedule in its direct chat with the user, after Grok Bot's routines: a morning brief, an hourly check, a weekly report. The bot owns its routines: the user asks for one in chat and the bot sets it up with the `routines` tool (a name, a schedule, and the prompt, written as an instruction to itself), and edits, pauses, resumes, runs, or deletes it the same way. The system prompt of every turn explains routines and lists the bot's own with each one's next run.

The roster also carries Auto-review (see Plugins). Routines live in the roster (`Routine { id, bot_id, name, prompt, schedule, is_enabled, enabled_at, last_run_at?, last_outcome?, paused_reason?, created_at }`), so every paired Device lists them and can pause, resume, run, or delete one (`routines.create` / `routines.update` / `routines.delete` / `routines.run`, and `routines.describe` to read a schedule back). The bot's Runner runs them (`crates/cli/src/routines.rs`): every half minute it starts the routines of its bots whose next run is due, at most one run per routine at a time.

A schedule (`crates/cli/src/schedule.rs`) is `every 30m`, `every 2h`, `every 1d`, or five cron fields read in the Runner's local time (`0 9 * * 1-5`), never more often than every five minutes. An interval counts from the last run, or from when the routine was created or resumed; a cron fires at the next matching minute after that. The CLI says a schedule in words on the wire (`schedule_text`: "Weekdays at 9:00 AM", "Every 2 hours", "On the 1st of every month at 8:30 AM", else "Cron 5 4 * * 1-3") and gives `next_run_at` and `is_running` beside every routine.

A run is a `routine` Job in the bot's DM. It opens with a "Routine · Name" marker (a system notice carrying the routine id), then the turn runs with the routine's prompt as the task and a system prompt that says nobody is typing: the bot does the work, replies with what the user should know, or answers `PASS`, which leaves only the marker. Later turns rebuild the marker as `[Routine "Name" ran on its schedule. Task: …]`, so the bot can talk about a run afterwards; the daily log names the routine as the turn's source. `job.started` / `job.finished` carry `routine_id`, so the apps show the routine as running and the bot at work. A run's outcome (`sent`, `pass`, `error`) is kept on the routine. Run Now from a Device that is not the Runner seals the job to the Runner like any turn.

When the user has not written in any chat for seven days, due routines are paused instead of run (`paused_reason: away`) with a notice in the bot's DM, as Grok Bot does, so nothing keeps spending on results nobody reads; the switch turns them back on.

The Mac app's DM inspector has a Routines section: a row per routine (clock, pause, or running icon; the schedule in words and the next run; a switch that pauses or resumes) that opens a sheet with the state, schedule, next and last run, the prompt, and Run Now, Pause/Resume, Edit in Chat (which puts `Edit my routine "Name": ` in the composer), and Delete. With none, the section says routines are set up by asking the bot. The phone's chat details show the same list with a switch; a tap offers Run Now and Delete.

### Plugins

A plugin is an MCP server (or several) a bot can use, after Grok Bot's marketplace: GitHub, Linear, Notion, Sentry, Context7, a headless browser, or any server the user pastes as JSON. A plugin is described by a manifest (`crates/cli/src/plugins/mod.rs`): `id`, `name`, `description`, `icon`, `servers` (`stdio` with `command`, `args`, `env`, or `http` with `url`, `headers`, and `auth`: `oauth` or `bearer`), `variables` (setup fields, `secret` ones never read back), `skills` (notes written to the plugin's folder that the prompt points the bot at), and `tools` hints (`readonly` patterns, `hide`). `${VAR}` in args, env, and headers is filled from the variables.

**Installed per Runner, for every bot there.** The Runner holds the install under `~/.lorca/plugins/` (`installed.json`, the plain variables; `secrets.json`, mode 0600, the secret variables and OAuth tokens; a folder per plugin with its skills) and advertises each plugin's id, name, and state (`ready`, `needs_setup`, `needs_auth`, `connecting`, `error`) in its `machine` blob, so every Device lists what each Runner has without a secret leaving it. Every bot on the Runner may use every plugin installed there, as every Grok Bot agent draws on the account's installs. Installing, removing, setting variables, signing in, and reading a plugin's detail run on the Runner: locally when the app's CLI is that Runner, else as a `request` sealed to it (`plugins.install`, `plugins.uninstall`, `plugins.variables`, `plugins.connect`, `plugins.detail`), so a phone installs a plugin on a Mac and hands it a key through ciphertext.

An installed marketplace plugin follows the index: at startup the Runner replaces the manifest it installed with the bundled one when that changed, and again when the marketplace loads, keeping variables, secrets, and sign-ins, so a new sign-in method or server reaches existing installs without a reinstall. A plugin added by hand is left as it is.

**The marketplace** is a JSON index of manifests: the one bundled in the CLI (`crates/cli/marketplace/index.json`) plus the one at `marketplace_url` in settings (or `LORCA_MARKETPLACE_URL`), fetched at most hourly. `plugins.marketplace { query? }` answers with each entry and the Runners that have it. An MCP config pasted in the app (`{ "mcpServers": { … } }` or one server) becomes a manifest of its own (`plugins.install { runner_id, name, mcp_json }`).

**At turn time** (`crates/cli/src/plugins/mcp.rs`, `rmcp`), the Runner connects each installed plugin's servers on first use and keeps them in a pool (stdio children, or streamable HTTP with the plugin's headers, a pasted token, or the saved OAuth tokens, refreshed by the transport and written back), lists their tools, and offers them to the model as `<plugin>__<tool>` with the server's instructions and the plugin's skills in the system prompt. A tool the server marks read-only (`readOnlyHint`) or the manifest lists as such runs at once. Any other tool goes through **Auto-review** (`crates/cli/src/plugins/review.rs`), after Grok Bot's: the setting lives in the roster (`AutoReview { is_enabled, rules[{ id, text, behavior: allow | ask, tool? }] }`, `auto_review.set`), so every Device shows and edits the same one. A rule made from a card's Always allow carries the exact `plugin/tool` and decides at once. Otherwise, with Auto-review on, the bot's own model (thinking off, 200 tokens) judges the one action against the user's natural-language rules, the built-in checks (deleting or overwriting, posting where others see it, money, access, bulk or irreversible changes ask; contained reversible work the user asked for runs), and the user's latest message, answering `allow` or `ask` with a reason; an ask rule wins over an allow rule. With it off, every such action asks. When the action asks, the bot posts a `permission` message in the chat ("Chef wants to use GitHub · create_issue · repo: …", with Auto-review's reason) and the turn waits up to ten minutes for `chats.permission { decision: allow | always | deny }` from any Device (sealed to the Runner when answered elsewhere); `always` adds the exact rule, `deny` gives the model a refusal it must not retry, and a routine run, with nobody there, refuses the call outright, so a routine only writes through what Auto-review or a rule allows. The working row reads "Using GitHub…" while a plugin tool runs.

**Sign-in** for a remote server is the MCP authorization flow run on the Runner by `rmcp`: discovery from the server's challenge, dynamic client registration as a native app, PKCE, and a loopback redirect on a free port; the browser opens on the Runner, so a Device that is not the Runner is told to finish there. It starts from a card in the chat, as in Grok Bot: after an install that needs a sign-in, or when the bot calls `connect_plugin`, the bot posts a `permission` message with `tool` `connect` ("Chef needs a sign-in to GitHub · Sign in · Not now"); Sign in answers it with `chats.permission { decision: allow }`, which the Runner turns into the flow, and the card then reads "Finish signing in in the browser on Workbench", "Signed in", or "Sign-in failed: …" (`decision` `allowed`, `connected`, `failed`). The inspector's plugin sheet offers the same sign-in. Discovery is seeded from the server's own 401 challenge, since GitHub keeps its resource metadata under the server's path. A manifest can name a `token_variable` so a pasted token stands in for the sign-in, and `client_id_variable` / `client_secret_variable` (or fixed `client_id` / `client_secret`) for a server that registers no clients on the fly; the card says what to fill in when nothing is set. With a `client_id` and a `device_authorization_endpoint` plus `token_endpoint`, the sign-in is the device flow (RFC 8628) instead: the Runner asks for a code, the card shows it with the link ("Copy code and open github.com" on the Mac, the same on the phone), the Runner polls until the code is entered, and the token is used as a plain bearer since such tokens carry no refresh token. Nothing opens on the Runner's screen, so the card works from any Device, and no client secret ships. GitHub's entry uses the device flow with Lorca's own OAuth app; a saved sign-in whose server metadata cannot be found again also falls back to a bearer. `LORCA_OAUTH_NO_BROWSER=1` fetches the authorize page instead of opening a browser, for tests against a fake server.

**The bot** can search the marketplace (`search_plugins`) and, when the user agrees on a permission card with `tool` `install`, install a plugin on its own Runner (`install_plugin`), as Grok Bot's InstallPlugin does after a question. It is told in the result what setup the user still owes (a sign-in, a variable).

**The apps.** The Mac app's DM inspector has a Plugins section: a row per plugin the bot's Runner has, with its state, and "Add from Plugins…", which opens the marketplace sheet (search, Install per row with the Runner's state, "Add MCP Server…" for pasted JSON). A plugin's sheet shows its state, the sign-in per OAuth server, the variables (secret fields write only), the Always allowed rules for its tools with Reset, its skills, and Remove. The Device pane lists what that Runner has installed. Settings → Auto-review has the switch ("Lorca checks each action before it runs and asks you first when needed") and the rules: "When a bot wants to: …" with Allow automatically or Ask first, Add rule, and delete. The transcript shows the permission card with Allow once, Always allow (not for installs), and Deny, and the reason when Auto-review paused the action. The phone shows the same card, the Runner's plugins in chat details, and Auto-review in its settings.

### Memory

Every bot keeps its own long-term memory on its Runner, in Grok Bot's shape of a curated profile over append-only logs, as plain markdown under `~/.lorca/workspaces/<bot id>/` (keyed by id, so a rename or a changed working directory never moves it; `crates/cli/src/memory.rs`):

- `MEMORY.md` is the curated memory. Its first 200 lines or 24 KB, whichever cuts first, open in every turn's system prompt; past that nothing loads, and the prompt tells the bot how much is cut off and asks it to consolidate. The bot writes it only through `memory_update`, one dated entry per call (`- 2026-09-17 · from your chat with the user · the user prefers short replies`), never a whole-file write, so two of its turns cannot clobber each other: `append` adds a fact (an exact duplicate is refused), `replace` rewrites one unique passage, `supersede` strikes the old entry through with a date and adds the new fact so the file shows what changed, `remove` deletes a passage.
- `memory/<topic>.md` files hold longer notes the bot reads with `read` when it needs them; the prompt lists their names.
- `memory/log/YYYY-MM-DD.md` is the bot's diary: what happened, not what is true. The Runner appends one line per finished turn (`- 09:05 · in group "Standup" · said "Sent the three flagged invoices…" · used bash, edit`), and the bot adds events with `memory_log`. Logs are never loaded into a prompt; `recall` finds them.

Every write is atomic and goes through a credential scrubber first (API keys, AWS and GitHub tokens, bearer tokens, JWTs, private key blocks, `password: …` values become `«redacted N chars»`), since memory is bot-authored as often as person-authored.

Two things keep a bot's chats from being islands, after OpenMausBot. Every turn's system prompt carries a **recent-work brief**: the newest thing the bot said in each of its *other* chats over the last two days, at most ten lines and about 350 tokens, newest first (`- today 09:05 · group "Standup" · you said: "…"`), so a bot in a group knows what it did in its DM an hour ago without the transcript. And `recall` searches the memory files and every chat the bot is in by words and by time, naming the chat and the speaker of each hit.

Before a compaction summarizes part of a chat, a silent **memory flush** turn runs over the messages about to be cut, with only `memory_update` and `memory_log` as tools and a prompt to save what is durable and not yet in memory; nothing it says reaches the chat, a failure or a two-minute timeout is logged and the compaction goes ahead (`LORCA_MEMORY_FLUSH=0` turns it off). This is what keeps a fact from being lost because it was only ever said in conversation.

The Mac app's DM inspector shows the bot's memory (`bots.memory`): the index against its load budget, an editor (`bots.memory.write`, refused with the bot's current text when the file changed under the editor, then Reload or Overwrite), and a Show button for the folder. For a bot on another Runner the CLI asks that Runner through the relay (a `request` / `response` pair, see Protocols), so the same rows read and edit it; only the folder cannot be opened, and an offline Runner shows as such with a Retry. The phone shows no memory yet.

### Providers

**DeepSeek** — API key on this Runner, streamed through DeepSeek's Anthropic-compatible endpoint (`https://api.deepseek.com/anthropic`, `providers::anthropic`), the one that runs DeepSeek's web search on the server: every request declares Anthropic's `web_search_20250305` tool, and the model's searches stream back as `server_tool_use` and `web_search_tool_result` blocks. `LORCA_DEEPSEEK_MODEL` overrides the model. The API root is the credential's own base URL when the user set one, else `LORCA_DEEPSEEK_BASE_URL`, else DeepSeek's (the key check calls its `/models`, bots its `/anthropic`; a root given with `/anthropic` already is used as is).

**Anthropic** — API key on this Runner, the Messages API (`providers::anthropic`): `claude-opus-5` by default with adaptive thinking, and Anthropic's `web_search_20260209` and `web_fetch_20260209` tools on every request (the basic variants and no thinking parameter for Haiku 4.5 and the 4.5 generation). `LORCA_ANTHROPIC_MODEL` overrides the model; the API root is the credential's base URL, else `LORCA_ANTHROPIC_BASE_URL`, else Anthropic's.

An API-key credential (`credentials.json`: `api_key`, `base_url?`, `connected_at`) can name a custom API root, for a proxy or a compatible server: `providers.connect_deepseek` / `providers.connect_anthropic` take `base_url` beside `api_key`, check the key against that root, and store both. The Connect sheet has the field, prefilled from the Runner's current credential; `providers[].base_url` and a `key · url` detail report it.

Both stream the same way, with the conversation prefix marked for caching (`cache_control` on the system prompt, the last tool, and the last user block), tool arguments streamed eagerly, and a request that fails before it streams retried twice: text, thinking (with its `signature`), `tool_use` blocks, and the server tools' own blocks. A server tool call becomes a `ServerToolStart` / `ServerToolEnd` pair for the Runner's activity rows, and the raw `server_tool_use` and `*_tool_result` blocks stay in the assistant message as `ServerBlock` parts, so when the same turn continues after a function call the model gets its searches, their results, and its sealed thinking back verbatim. A later turn's context is rebuilt from the chat and carries none of it.

**ChatGPT** — subscription OAuth on this Runner (`providers::chatgpt`, isolated): authorization code with PKCE, the localhost:1455 callback the Codex CLI uses, tokens refreshed by the adapter, and the Codex responses backend for streaming. `LORCA_CHATGPT_MODEL` overrides the model. Every request carries the backend's built-in `web_search` tool, which searches and reads pages server-side; its `web_search_call` items stream back as `ServerToolStart` / `ServerToolEnd` events (`web_search` with the query, `web_fetch` with the URL for an `open_page`).

**Grok** — subscription OAuth on this Runner (`providers::grok`, isolated): xAI's authorization code with PKCE at `auth.x.ai` (`/oauth2/authorize`, `/oauth2/token`) with the public desktop client id the Grok CLI uses, a loopback callback on a port picked at sign-in (`http://127.0.0.1:<port>/callback`; the `accounts.x.ai` page calls it from JavaScript, so the callback answers the CORS preflight and marks its response for that origin, else the page falls back to a code to paste), and tokens refreshed by the adapter (access tokens last about six hours; the refresh token rotates and the new one is stored at once). It needs a SuperGrok or X Premium+ account. Streaming is xAI's Responses API (`https://api.x.ai/v1/responses`) with the bearer; the Responses wire shape (`providers::responses`) is shared with ChatGPT. `grok-4.6` by default; `LORCA_GROK_MODEL` overrides it, `LORCA_GROK_BASE_URL` the API root, `LORCA_GROK_ISSUER` the OAuth issuer, for a test server. Every request carries xAI's `web_search` and `x_search` tools, run server-side; their `web_search_call` and `x_search_call` items stream back as `ServerToolStart` / `ServerToolEnd` events ("Searched the web for …", "Searched X for …", "Read …"). Disconnecting revokes the refresh token at `auth.x.ai`, best effort.

Server tool events from any provider become tool rows on the Runner, so the status line reads "Searching the web…" or "Reading the web…" while one runs; `build_context` never replays those rows to the model.

The encrypted bot profile carries `provider`, `model`, and `thinking` as labels; the Runner resolves them against its own credentials and the model catalog at turn time. `thinking` is one of `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (or unset for the provider's default) and is mapped to what the model takes: an `output_config.effort` with adaptive thinking on DeepSeek and current Claude models (Fable cannot turn thinking off, so `off` becomes its lowest level), a `budget_tokens` on Haiku 4.5, `reasoning.effort` on ChatGPT, and `reasoning.effort` (`low`, `medium`, `high`) on the Grok models that take it (4.3, 4.5, 4.6); the others reason on their own and get no parameter. Every bot tool that creates or edits a teammate takes `thinking` too.

**The model catalog** (`lorca_agent::models`, a snapshot of models.dev) knows each offered model's context window, output cap, image support, thinking levels, and rates. Every assistant message carries `usage.cost` in dollars from those rates (ChatGPT and Grok sign-ins are not billed per token; their cost is what the work would cost at API rates). The Runner adds each turn to the chat's `usage` (`context_tokens` and `context_window` of the last turn, totals of tokens, cost, and turns), kept in `state.json` and never synced; the app gets it in the snapshot, in `roster.changed`, and as `chat.usage` events, and the DM inspector shows "Context 128k of 1M · 13%" and "Spent $0.42 · 18 turns".

**Compaction** keeps a long chat inside the window (`lorca_agent::compaction`, pi's prompts and cut rule; `LORCA_COMPACTION=0` turns it off): when the transcript a turn rebuilds is estimated above `window - 16k` tokens, or more than 400 messages of the chat are not covered by a summary yet, or a turn's own context grows past the window between model calls (the loop's `prepare_next_turn` hook), or the provider answers that the prompt is too long, the older part is summarized by the same model into a structured checkpoint (goal, progress, decisions, next steps, files touched), about 20k tokens of recent messages stay as they are, and a `Compaction { bot_id, summary, after_message_id }` on the chat (local, per bot) makes every later turn start from the summary and the messages after that one. The summary covers the whole chat since the last one, in pieces of at most the window less twice the reserve, each piece updating the summary of the ones before, so a backlog of any size compacts. The memory flush runs first (see Memory). Automatic compaction is silent, as in Grok Bot; the inspector's Context row shows the result, and its Compact button (`chats.compact`) does it by hand with a notice in the chat.

**Retries**: a model call that fails before it streams in a way that reads as transient (overloaded, rate limited, 5xx, a dropped connection) is asked again after a backoff, three times at most on top of the adapters' own HTTP retries; the app hears `job.retry` and the working row reads "Retrying (2 of 3) in 4 s…". A failure that outlasts the retries is the bot's failed bubble, as before.

## macOS app

SPM `Lorca.app`, AppKit.

The app starts the bundled `lorca` (Contents/MacOS/lorca; `LORCA_CLI` overrides, PATH is the fallback) as `lorca serve --port <port>`, logs it to `~/Library/Logs/Lorca/cli.log`, and restarts it if it exits. If something already listens on the port, that instance is used.

First run: the CLI answers `hello` with `has_identity: false`, and the app shows onboarding: create (the CLI returns the phrase), restore (phrase → relay), or pair (paste the string from the identity Mac). After create, the user names the first bot (the CLI's default Chef, edited through `bots.update`), then connects a provider on this Mac (a DeepSeek or Anthropic key, or a ChatGPT or Grok sign-in; skippable). Restore and pair go straight to the provider step, since the roster syncs.

Settings is a mode of the main window. The foot of the sidebar has two icons: the gear opens General (as ⌘, does), and this Mac's icon, red while the CLI is not answering, opens its Device page there. The sidebar gives way to the settings sidebar: Back (or Escape) on top, a search field, the panes (General, Providers, Auto-review, Advanced), and the paired Devices, each opening its Device page in the content area; the Devices header carries Pair a Device. A query (⌘F focuses the field) narrows the list to the panes, settings, and Devices that match, each setting under its pane; picking a setting opens the pane, scrolls to its row, and flashes it. The index is `SettingsEntry` (`Settings/SettingsSearch.swift`): a title, keywords, and the row label the panes take their labels from, so the results and the pages share one spelling. Both sidebars use a standard `NSSearchField`, which brings AppKit's capsule and focus ring. From macOS 26 a sidebar's list fills the pane and scrolls under everything in it, as System Settings' does: the bars (the chats' search field and footer, Settings' Back and search field) are `NSSplitViewItemAccessoryViewController`s the root sets on the sidebar's split view item for whichever sidebar shows, so AppKit insets the list by them through the safe area and softens the rows passing behind them and the titlebar. On macOS 14 and 15 the bars and the list are stacked inside the pane (`SidebarChrome.floats`). The selection says which sidebar shows (`Selection` is `chat`, `settings(pane)`, or `device`), so opening a Device from the inspector lands in Settings too, and Back returns to the chat that was open. The panes show while the CLI is not answering, since the CLI port and the relay URL live there. While onboarding is up there is no main window, and ⌘, opens General and Advanced in a small window of their own.

Chrome: split view, vibrancy, bubbles, `@` mentions. The titlebar is AppKit's own: the transcript, the settings panes, and the Device pages run their scroll views under it, so the header has no background at rest and gets the scroll-edge effect and its separator once content is beneath it. The app renders CLI events and applies its own edits optimistically; `LORCA_MOCK=1` runs the seeded demo instead. Keys stay in the CLI.

Composer: the `+` button opens a file panel; files dropped on the composer or pasted (file URLs, or an image with no text beside it, written to a temporary PNG) attach the same way. Chips above the text show a thumbnail or the file's name and size, each with a remove button; a message can be attachments alone. The app mints the attachment ids and hands the CLI paths (`chats.send { attachments: [{ id, path, name, mime, width, height }] }`), so the bubble it shows at once matches the message the CLI echoes back. In a bubble, images are thumbnails sized from the width and height in the message and files are cards; a click opens the file. An attachment from another Device is fetched through `files.path`, which pulls the `file` blob from the relay into `~/.lorca/files/` and answers with the path; the bubble reloads when it lands. The Dictate button (`mic.fill`, the primary disc while the field is empty) starts Apple's speech recognizer (`Speech` + `AVAudioEngine`, `Dictation.swift`). While recording, the text gives way to Grok Bot's pill: a stop square, the elapsed time, and bars that follow the input level, with Send still beside it. The transcript accumulates and lands at the caret when the square is clicked; Send commits it and sends in one go; Escape discards it. The recognizer listens in the language of the keyboard input source in use (a Pinyin input method means zh-CN, whatever the system language), else the first of the system's preferred languages it supports (matched on language and region, so "zh-Hans-CN" finds "zh-CN"), or the language chosen in Settings › General or the button's right-click menu (`Preferences.dictationLanguage`). The bundle declares `NSMicrophoneUsageDescription` and `NSSpeechRecognitionUsageDescription`.

Working state, after Grok Bot: the CLI's `job.started` / `job.finished` events (and `running_turns` in the snapshot) name the chat and bot of every turn in flight, including a turn sent to another Runner. From them the app shows a breathing green dot on the bot's avatar in the sidebar, an "is working" row after the last message (the avatar alone in a DM, "Chef is working…" or the bot's latest tool as an activity such as "Running commands…", kept between calls so it does not flash, in a group), the Stop button, and "Chef stopped without replying" when a turn ends with nothing said. Replies arrive in chunks: the bubble appears with the first completed paragraph (or sentence, after 1.5 s) and grows by paragraphs, the way Grok Bot's server re-sends a message as it grows. Tool calls never appear in the transcript, as in Grok Bot: the CLI keeps them as `tool` messages so a later turn can rebuild its context, and the app renders none of them, leaving the "is working" row (with the running tool's activity) and whatever the bot says before and after its work. The one exception is a sent `message_bot`, shown as the "Messaged ◉ X" marker. Sidebar rows carry a blue unread dot, a preview without a "You:" prefix (a bot's name only in groups; "Messaged X" and "Message from X: …" for bot-to-bot messages), and a stamp that is the time today, "Yesterday", the weekday within a week, then the date. A transcript inserts "Today 4:13 AM" separators after fifteen minutes of silence; in a group a bot's name sits above its bubble with its avatar beside the bubble's bottom edge, and a DM shows neither.

## Protocols

### App ↔ CLI (local WS)

JSON on `ws://127.0.0.1:4862/ws`. Requests are `{ id, method, params }` and get `{ id, result }` or `{ id, error: { message } }`; events are `{ event, data }`.

App → CLI: `hello`, `bootstrap`, `identity.create`, `identity.restore`, `pair.start` / `pair.status` / `pair.cancel` / `pair.accept`, `config.set`, `bots.create` (`runner_id` may be another Device; it must be a Runner) / `bots.update`, `chats.create` / `chats.dm` / `chats.send` / `chats.stop` / `chats.delete` / `chats.rename` / `chats.pin` / `chats.add_bot` / `chats.remove_bot` / `chats.mark_read` / `chats.messages` / `chats.compact`, `routines.create` / `routines.update` / `routines.delete` / `routines.run` / `routines.describe`, `plugins.marketplace` / `plugins.install` / `plugins.uninstall` / `plugins.set_variables` / `plugins.connect` / `plugins.detail` (`runner_id` names the Runner) / `auto_review.set` / `chats.permission`, `bots.memory` / `bots.memory.write` (this Runner's bots), `providers.connect_deepseek` / `providers.connect_anthropic` / `providers.connect_chatgpt` / `providers.connect_grok` / `providers.disconnect` (this Runner), `ui.watching` (the chat on screen in the desktop app), `push.register` / `push.unregister` (a phone's device token).

CLI → App: `snapshot`, `roster.changed` (devices, bots, chats, routines), `message.added` / `message.updated` / `message.removed`, `chat.removed`, `job.started` / `job.finished` (with `routine_id` for a routine's run) / `job.retry`, `chat.usage`, `relay.status`, `pair.completed`, `identity.changed`.

The app may choose ids (`bots.create.id`, `chats.create.id`, `chats.send.message_id`) so its optimistic rows match the CLI’s events.

What the apps get of a chat is a view of it (`Message::for_app`, `message_page` in `model.rs`). A snapshot carries each chat's newest 60 messages and `has_more` when older ones are left; `chats.messages { chat_id, before?, limit? }` answers the page before a message id, oldest first, with `has_more`. The Mac transcript asks for it as it scrolls within reach of the first row and holds what is on screen in place; the phone asks from FlashList's `onStartReached`. Both keep pages they loaded when a later snapshot arrives. A `tool` message reaches the apps, in snapshots, pages, and `message.*` events, with its name, summary, running state, and the first 400 characters of its detail, and without its arguments and result: the apps show none of that, and a file read or a command's output runs to hundreds of kilobytes. The Runner's `state.json` keeps them whole for the next turn's context.

### CLI ↔ relay

- Machine bearer for blobs and presence; identity signature for registering and attesting machines.
- PUT/GET blobs; body is ciphertext. `roster` is a whole-roster snapshot of bots, chats, and routines (latest wins); `chat` is one upsert or removal of a message; `machine` is a Device’s metadata; `key` is the DEK sealed to the content key.
- Jobs: `kind=job` with `recipient_machine_pubkey`, sealed to that machine’s box key, deleted by the Runner after the turn.
- Questions: `kind=request` sealed to one Runner (`crates/cli/src/requests.rs`: `{ id, verb, requested_by, body }`), answered with a `kind=response` sealed to the Device that asked (`{ request_id, body, error? }`); each side deletes the blob it consumed, and the asker gives up after 20 s. The verbs are `memory.read` and `memory.write`, so a bot's memory can be shown and edited from a Device that is not its Runner; `plugins.install` / `plugins.uninstall` / `plugins.variables` / `plugins.connect` / `plugins.detail`, so plugins on a Runner are managed from any Device; and `permission.answer`, so a permission card is answered from any Device. A request is refused up front when the Runner is unknown or offline.
- Every Device keeps `last_seq` and an outbox; uploads retry until the relay accepts them.

```
App_B → CLI_B
CLI_B encrypts user message + job-for-A → relay
CLI_A fetches envelopes for A, decrypts, runs loop, encrypts reply
CLI_B fetches chat blobs, decrypts → App_B
```

## Repo layout

```
lorca/
  ARCHITECTURE.md
  README.md
  Cargo.toml           # workspace
  crates/agent/        # lorca-agent: loop, tools, providers (Anthropic Messages for DeepSeek and Anthropic, ChatGPT, Grok)
  crates/cli/          # lorca: the Device core as a library (keys, relay sync, jobs, the JSON API) + runner and server features + the binary
  crates/mobile/       # lorca-mobile: the core for the phone over UniFFI
  crates/relay/        # lorca-relay: axum + SQLite
  macos/               # AppKit SPM app; the build bundles the CLI
  mobile/              # Expo app for iOS and Android: a paired Device over the core (modules/lorca-core)
  web/                 # the site
  scripts/             # bun scripts: dev loop, bundle build
```

`bun run mobile` starts the Expo dev server; `bun run mobile:ios` / `mobile:android` build and run the dev client; `cd mobile && bun run core` rebuilds the Rust core for both platforms first.

`bun run dev` rebuilds the CLI and the app on Rust or Swift changes (the generated markdown bindings under `macos/Sources/LorcaMarkdown` are left out of the watch, and rewritten only when they differ) and relaunches the app through `open`, so the app is its own responsible process for TCC: a binary spawned from the terminal is charged to the terminal app, whose Info.plist decides whether a microphone or speech request aborts. `bun run build` produces a release bundle. The bundle step restamps the app binary's SDK version (`stampSDK` in `scripts/app.ts`, through `vtool`): the Swift Build engine writes the deployment target (14.0) there, and AppKit gives a binary stamped below the macOS 26 SDK its older look, with a flat sidebar and an opaque titlebar strip. `bun run relay` runs a local relay.

## Status

Done: crypto and blob protocol, relay, CLI (identity, pairing, restore, local WS, DeepSeek and Anthropic keys, ChatGPT and Grok OAuth adapters, server-side web search, agent loop, encrypt-before-upload, group chats, cross-Runner jobs and handoffs, stop, routines, plugins over MCP with a marketplace and permission cards, encrypted pushes for finished replies), app wiring and the bundled CLI launcher.

Next: steering mid-turn, keychain storage, relay blob GC, a cost budget per chat.

The phone app (`mobile/`) pairs as a Device with `os` `ios`, `ipados`, or `android`; it is never a Runner and does not hold the master secret. The first Mac is the identity device.

## Open points

- Relay blob compaction / GC
- Model ids move: DeepSeek defaults to `deepseek-flash` (`deepseek-v4-pro` for reasoning), Anthropic to `claude-opus-5` (`claude-sonnet-5`, `claude-fable-5-1`, `claude-opus-4-8`, `claude-haiku-4-5` offered), ChatGPT sign-ins default to `gpt-5.6-terra` (`gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-luna`, `gpt-5.5` also accepted; `*-codex` ids are rejected for ChatGPT accounts), Grok sign-ins to `grok-4.6` (`grok-4.5`, `grok-4.3`, `grok-4.20-0309-reasoning`, `grok-build-0.1` offered). Each bot carries an optional `model` and `thinking` level (New Bot sheet, and the DM inspector's "Runs with" section); `LORCA_DEEPSEEK_MODEL` / `LORCA_ANTHROPIC_MODEL` / `LORCA_CHATGPT_MODEL` / `LORCA_GROK_MODEL` override the defaults for bots without one
- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.
