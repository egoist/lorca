# Tinybot Architecture

Source of truth for how Tinybot is built. Read this before writing code.

Tinybot is a Grok Bot alternative: persistent named bots, 1:1 chats, group chats, handoff, and orchestration. Work runs on **Devices you own**. The macOS app is the UI for the local Rust CLI, which it bundles and launches.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The app ships the CLI binary inside its bundle and starts `tinybot serve` itself, unless one already answers on the port. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM): system materials, SF Symbols, Auto Layout, keyboard, accessibility.
3. **The CLI owns the agent loop:** inference, tools, streaming, cancellation, orchestration.
4. **The relay is zero-knowledge:** opaque blobs and public keys. Auth is a signature challenge.
5. **Every Device records its `os`.** A Device with a desktop `os` (`macos`, `linux`, `windows`) is a **Runner**. Phones and tablets (`ios`, `ipados`, `android`) are Devices, never Runners.
6. **Provider credentials live on the Runner the bot is assigned to.** You can create a bot for any paired Runner. DeepSeek and Anthropic keys and ChatGPT tokens stay on that assigned machine.
7. **A bot runs on one Runner:** that Device’s CLI.

## Three processes

```
┌─────────────────────┐     local websocket      ┌──────────────────────────┐
│  Tinybot.app        │ ◄──────────────────────► │  tinybot CLI (Rust)      │
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
- **Relay:** store-and-forward API. Rust, axum, SQLite (`crates/relay`). Self-host it anywhere; clients point `TINYBOT_RELAY_URL` at it.

If the CLI is down, the app shows a native empty state with the launcher’s status and the manual `tinybot serve` command.

## Identity

An identity is keys you hold.

Mac-first, after Happy’s layering. Until a phone exists, the first Device is the identity device.

| Layer                    | What                                                                                | Where                                                                                                |
| ------------------------ | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Master secret (32 bytes) | Root. Backup as a base32 phrase (13 groups of 4).                                   | `~/.tinybot/identity.json`, mode 0600. Stays on the identity device.                                 |
| Content keypair          | X25519, HKDF from master. Secret unseals the DEK. Public key seals it.              | Secret on Devices that have the master. Public key on the relay.                                     |
| Identity signing key     | Ed25519, HKDF from master.                                                          | Private local. Public key on the relay; the identity id is `hash(pubkey)`.                           |
| Account DEK              | Random XChaCha20-Poly1305 key. Encrypts roster, chats, messages, machine metadata.  | Made locally. On the relay as a `key` blob **sealed** to the content public key; handed to each paired machine inside the sealed pairing reply. |
| Machine keypair          | One 32-byte secret per Device → HKDF → Ed25519 signing key + X25519 box key.        | `~/.tinybot/machine.json`. Public keys on the relay, attested by the identity.                       |
| Chat/job envelopes       | Account DEK for roster/chat/machine blobs; sealed box to the Runner’s box key for jobs. | Relay stores ciphertext.                                                                          |
| Ephemeral pairing key    | X25519, one handshake.                                                              | Devices; discarded after pairing.                                                                    |

AEAD envelopes are `nonce(24) || ciphertext` with the blob kind as associated data. Sealed boxes are libsodium `crypto_box_seal` (`crypto_box` crate); signatures are `ed25519-dalek`.

Recovery: restore the master secret from the backup phrase → re-derive content keys → unwrap DEKs from the relay. The backup phrase is the identity.

### Pairing a Device

1. Device A (has the identity) asks the relay for a pairing nonce and shows a pairing string: `tinybot://pair?relay=…&id=<identity pubkey>&ek=<ephemeral pubkey>&n=<nonce>`.
2. Device B pastes it (onboarding, or `tinybot pair <string>`). B generates its machine keys and posts a request sealed to `ek` into the relay’s pairing mailbox (`POST /v1/pair/{nonce}/request`, no auth): its machine public key, box public key, `name`, `os`.
3. A polls the mailbox, unseals the request, attests B on the relay with an identity-signed `POST /v1/identities`, and posts a reply sealed to B’s box key: identity public key, content public key, the **account DEK**, and the relay URL. Provider credentials stay on each Runner.
4. B unseals the reply, saves `machine.json`, authenticates with the challenge, and uploads its `machine` blob (`name`, `os`, connected provider kinds).
5. B syncs the roster and chats and shows up in the Device list. If B is a Runner, connecting a provider on B happens on B.

App ↔ CLI on one machine uses `127.0.0.1`; those keys are already local.

### Devices and Runners

Every Device writes its `os` into its machine metadata blob. Values: `macos`, `linux`, `windows`, `ios`, `ipados`, `android`. The client sets it at pairing and re-sends it with presence.

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
Bot      1──* Job          (a turn on the bot's Runner)
```

| Entity             | Device                                                    | Relay                                                  |
| ------------------ | --------------------------------------------------------- | ------------------------------------------------------ |
| Identity           | Master + content + signing keys                           | Public key                                             |
| Device             | Machine keypair, `os`, local provider creds (Runner only) | Machine public key + encrypted metadata blob           |
| Bot                | Decrypted profile                                         | Inside encrypted roster blobs                          |
| ProviderCredential | Assigned Runner’s keychain                                | —                                                      |
| Chat / Message     | Account/chat DEK                                          | Encrypted blobs                                        |
| Job                | Any paired Device may create; the assigned Runner runs it | Sealed envelope to that Runner’s machine box key; deleted once run. A `room_turn` answers with a `job_result` sealed to the requesting Device |

Creating a bot for Runner B from Device A: A writes an encrypted bot profile into the roster (paired Devices can read it) and pins B’s machine id. Bot create rejects a target whose `os` is not desktop. Turns are job envelopes addressed to B. B decrypts the job, runs the loop with B’s provider credentials, and uploads encrypted replies.

If B is offline or still connecting a provider, the envelope waits on the relay until B fetches it. The UI infers that from decrypted roster state.

## Website

`web/` is the app's site: TanStack Start (React, file routes under `web/src/routes`), Tailwind and shadcn/ui components, built with Vite and served by a Cloudflare Worker (`web/wrangler.jsonc`, `@cloudflare/vite-plugin`, static assets alongside the SSR entry). One page: hero, screenshots of the app in `TINYBOT_MOCK=1` mode (`web/public/screens`), features, how it works, privacy, FAQ. `bun run web` serves it locally on port 3000; `bun run web:deploy` builds and runs `wrangler deploy`.

## Phone app

`mobile/` is an Expo app (React Native, expo-router, TypeScript) for iOS and Android. A phone has no CLI, so the app speaks the relay protocol itself: `mobile/src/core` is a TypeScript port of the CLI's Device role, byte-compatible with `crates/cli` (a test in `crypto.rs` checks vectors the port produced).

- **Keys and crypto** (`keys.ts`, `crypto.ts`): the machine secret in the secure store (Keychain / Keystore); HKDF to the Ed25519 signing key and X25519 box key; XChaCha20-Poly1305 envelopes keyed by the account DEK with the blob kind as associated data; libsodium sealed boxes for jobs, pairing, and job results.
- **Pairing** (`pairing.ts`): the phone scans the QR code the Mac shows (or pastes the string), posts a request sealed to the ephemeral key, polls for the reply sealed to its box key, and keeps the account DEK and relay URL in the secure store. Its `os` is `ios`, `ipados`, or `android`.
- **Sync** (`engine.ts`): the CLI's relay loop, in the app: bearer from the signature challenge, outbox, presence, a 25 s long-poll, blobs applied in sequence order (roster, chat ops, machine metadata, job results). Plaintext state lives in one JSON file in the app's documents directory, like the CLI's `state.json`. The loop pauses in the background and resumes on foreground.
- **Attachments**: the composer's `+` offers the photo library, the camera, and the file picker. A picked file is read into the app's `files/` directory, encrypted with the account DEK as a `file` blob under the attachment's id (ciphertext waits on disk beside the outbox, not inside state.json), and uploaded ahead of the message that names it. The poll leaves `file` blobs out; a bubble that shows an attachment this phone does not have fetches it by id. Images render as thumbnails sized from the width and height in the message (full screen on tap); other files as a name-and-size card.
- **Dictation**: the primary disc is Dictate while the field is empty (a small microphone sits inside the field once there is text). While recording, the field shows Grok Bot's pill: a stop square, the elapsed time, and bars that follow the microphone, with Send still beside it. The words land in the field after whatever was typed when the user taps the square, or go out at once when the user taps Send. The recognizer (`expo-speech-recognition`) listens in the first of the phone's preferred languages it supports (`src/ui/dictation.ts`; a Chinese speaker on an English-region phone gets zh-CN), or the language chosen in Settings or by a long press on the microphone.
- **Turns**: sending in a DM uploads the message and a `turn` Job sealed to the bot's Runner; a group message runs the room exchange from the phone, exactly as `run_room` does: `room_turn` Jobs in order, mentions first, rounds while anyone spoke, winding down on the fourth. The Runner answers every Job the phone requested with a `job_result` sealed to the phone, which is how the phone knows a turn ended (and shows "Chef stopped without replying" when nothing was said). A later message in the same group waits for the running exchange, the chat lock a Runner holds. There is no Stop control, as in Grok Bot: a turn runs to its end.
- **Roster edits**: new bots (for a paired Runner), groups, rename, pin, members, delete are written as roster blobs, like the CLI.

The UI is native: a native stack with large titles, search, and toolbar items; a composer after Grok Bot's phone app, a liquid-glass `+` button and glass pill (`expo-glass-effect`, a filled pill where glass is unavailable) floating over the transcript with the Dictate or Send disc inside the pill's right edge, riding the keyboard as a `KeyboardStickyView` while the transcript's `KeyboardChatScrollView` lifts the last messages with the keys by growing its bottom inset, so the list is never resized; form sheets for chat info, new bot, new group, and settings; Link previews and context menus on chat rows; SF Symbols (Material Symbols on Android); system colors; haptics. The transcript follows the Mac app: bubbles with the bot's name above and its avatar beside the bubble's bottom edge in a group, neither in a DM; "Today 4:13 AM" separators after fifteen minutes; the "is working" row and the breathing green dot on avatars; "Message from ◉ X" and "Messaged ◉ X" markers; tool calls never shown; sidebar-style previews and stamps.

## Credential locality

- **Provider setup** on that Runner (keychain or `~/.tinybot/credentials`, mode `0600`).
- **Bot create** may target any paired Runner. The relay payload is ciphertext of the profile.

## Relay

`crates/relay`: Rust, axum, rusqlite (bundled SQLite). `tinybot-relay --bind 127.0.0.1:8787 --db tinybot-relay.db`; `TINYBOT_RELAY_SECRET` signs bearer tokens (random per boot when unset).

Auth is per machine:

1. `POST /v1/auth/challenge { machine_pubkey }` → nonce.
2. Client signs the nonce with the machine signing key: `POST /v1/auth/verify`.
3. Relay issues a one-hour HMAC bearer bound to the identity and machine.

Identity-level writes are signed requests: `{ payload: base64url(json), signature }` where the payload carries `identity_pubkey` and `ts` and the identity key signed the bytes. `POST /v1/identities` registers the identity (idempotent) and attests one machine; it is used at create, restore, and for every pairing.

Tables:

- `identities(pubkey, content_pubkey, created_at)`
- `machines(machine_pubkey, identity_pubkey, box_pubkey, attestation, last_seen, created_at)`
- `blobs(id, identity_pubkey, kind, recipient_machine_pubkey nullable, seq, ciphertext, size, created_at)`
- `sequences(identity_pubkey, seq)`, `challenges`, `pairings(nonce, identity_pubkey, request, reply, expires_at)`

`kind` is `roster` | `chat` | `job` | `job_result` | `machine` | `key` | `file`. Ciphertext is bytes; the nonce sits inside it. `seq` increases per identity. A Device’s `name` and `os` are inside its `machine` blob, not columns. A `file` blob is an attachment's bytes under the attachment's id, up to 24 MB of ciphertext (other kinds 4 MB); Devices poll with an explicit kinds list that leaves `file` out and fetch one by id when a transcript needs it.

Blob API: `PUT /v1/blobs` (client-chosen id, idempotent), `GET /v1/blobs?since=<seq>&kinds=&wait=25` (long-poll; returns blobs for the identity that are unaddressed or addressed to the caller’s machine), `GET /v1/blobs/{id}` (one blob, same visibility), `DELETE /v1/blobs/{id}`, `GET /v1/machines` (presence).

Clients set `TINYBOT_RELAY_URL` or the relay URL in Settings › Advanced. Without a relay the CLI works on one Device alone. In dev (`bun run dev` sets `TINYBOT_DEV=1` and runs a relay on `0.0.0.0:8787`), a Device with no relay configured defaults to `http://<this Mac's LAN IP>:8787`, so Pair a Device shows a code a phone on the same network can use.

A CLI lists blobs for its identity and envelopes for its machine public key (long-poll), decrypts, and emits events to the app on localhost.

## CLI (runtime)

`crates/cli`, binary `tinybot`. Local websocket for the app. Files under `~/.tinybot/` (override with `TINYBOT_HOME`), all mode 0600: `identity.json` (master secret, identity devices only), `machine.json` (machine secret, identity and content public keys, account DEK, `name`, `os`), `credentials.json` (this Runner’s providers), `settings.json` (relay URL), `state.json` (plaintext roster, chats, sync cursor, outbox), `files/<attachment id>` (attachment bytes, sent from here or fetched from the relay).

Commands:

- `tinybot serve` — default; the app connects here
- `tinybot identity new` / `identity restore <phrase>` / `identity show`
- `tinybot pair` — show a pairing string and wait; `tinybot pair <string>` joins
- `tinybot status` / `doctor`

Bind: `127.0.0.1:4862` (`--port`, `TINYBOT_PORT`). Relay: `TINYBOT_RELAY_URL`. If the port is busy the CLI fails loudly.

### Agent loop

`crates/agent` (`tinybot-agent`) is a port of pi-agent-core: `run_agent_loop` / `run_agent_loop_continue`, the same event sequence (`agent_start`, `turn_start`, `message_start/update/end`, `tool_execution_start/update/end`, `turn_end`, `agent_end`), steering and follow-up queues, `before_tool_call` / `after_tool_call` / `should_stop_after_turn` hooks, sequential or parallel tool execution, and `terminate` hints from tools. A `Provider` turns a request into a stream of assistant events and never fails: errors become an assistant message with `stop_reason` `error` or `aborted`.

```
on decrypted Job:
  build context from the chat (this bot's turns are assistant, other bots' text is user "[Name]: …",
                               this bot's tool rows become tool_call + tool_result pairs)
  run_agent_loop_continue(context, provider = this Runner's creds, tools, sink)
    events → job.started / job.finished (chat, bot) → local app WS, which shows the bot at work
           → a reply grows in chunks, not tokens: the text so far is re-sent at paragraph ends,
             or at a sentence end after 1.5 s of silence, never mid-word; the end sends it complete
           → every chunk and every completed message is encrypted with the account DEK → relay,
             so a paired phone watches the reply grow the way the local app does
    tool calls execute
```

Turns in one chat run one at a time on a Runner. `chats.stop` cancels the running turn (and the room exchange it belongs to).

### Tools

Team tools (the CLI):

- `message_bot { bot, message }` — a message to a bot outside the current chat. The caller's chat shows a "Messaged ◉ X" marker; the target's own DM gets a "Message from ◉ X" marker and a `message` Job (with `from_bot_id` and `hops`) whose envelope goes to the **target bot’s Runner**. The tool refuses a member of the same group: they read that chat and take their own turn.
- `list_teammates` — decrypted local roster with Runner and online state.
- `remember { note }` — appends to the bot’s memory file.
- `create_bot { name, label, description?, instructions, provider?, workdir? }` — a new teammate on the caller’s Runner with its own DM; in a group chat it joins that chat. This is how a lead bot builds its team.
- `edit_bot { bot, name?, label?, description?, instructions?, provider?, workdir? }` — changes a teammate’s profile, or the caller’s own. Only the passed fields change and instructions replace in full; the new profile applies from that bot’s next turn.

Coding tools (`tinybot_agent::tools`, ports of pi’s built-ins, same schemas and truncation rules: 2000 lines / 50KB, whichever first):

- `read { path, offset?, limit? }`, `write { path, content }`, `edit { path, edits: [{ oldText, newText }] }`
- `bash { command, timeout? }` — tail-truncated, full output to a temp file, process group killed on stop
- `grep { pattern, path?, glob?, ignoreCase?, literal?, context?, limit? }`, `find { pattern, path?, limit? }`, `ls { path?, limit? }` — respect `.gitignore` through the `ignore` walker, no `rg`/`fd` needed

Every bot has a working directory on its Runner (`workdir`, default `~/.tinybot/workspaces/<bot id>`, so a rename never moves files); relative paths resolve there.

### The lead bot

A new identity starts with one bot, **Chef**, a chief of staff on the first Mac: an ordinary bot with a default profile whose instructions are to learn the user’s work, propose a small team of one-job bots, create them with `create_bot`, and route work with `message_bot`. Nothing about it is privileged; rename or delete it like any bot.

A chat has a `kind`. A DM is one bot and never gains or loses members; there is one DM per bot. A group holds one to six bots, can add or remove them after creation, and has an `owner_bot_id` (the bot holding the work; defaults to the first member, `chats.set_owner` changes it).

Who answers, after Grok Bot's rooms:

- **DM:** its bot, always. An `@Name` is a reference the bot acts on: it calls `message_bot`, which delivers the message into that bot's own DM with the user, where that bot answers (and can message back).
- **Attachments:** a user message can carry files. Their bytes travel as `file` blobs; before a turn the Runner fetches any it lacks (`files::prefetch`), copies each into the bot's workspace at `attachments/<attachment id>/<name>` (a stable path, so every turn names the same file), and the transcript's user message gets a line per file naming that path, plus the pixels as an image content part for an image up to 5 MB, so a vision-capable model sees it and any model can open it with its tools.
- **Group:** a room exchange (`run_room`). The Device that received the user's message offers every member a turn, one at a time, in chat order with the members the message names by `@` first. A member's turn is a `room_turn` Job with the whole transcript plus an ephemeral cue (round number, how many messages are new to it); the member replies to the group or answers `PASS`, which never becomes a bubble. A round with at least one reply is followed by another, offered only to members who have heard something new; the exchange ends after a silent round or after the fourth round, which is marked winding down so members add only what is essential. The user's next message in that chat waits for the exchange (the chat lock); `chats.stop` cancels it. A member on another Runner gets its job through the relay and reports back with a `job_result` blob (`sent`, `pass`, `error`) sealed to the requesting Device; the room waits up to five minutes for it and skips an offline Runner.
- **Bot to bot:** `message_bot` from any chat to a bot outside it; the message lands in the target's DM. Each hop carries `hops`; after eight bot-to-bot hops without a user message the tool refuses, so two bots cannot loop. Members of one group talk to each other in the group.

Every bot keeps its own memory (`workspaces/<id>/MEMORY.md`, written by the `remember` tool, shown at the start of every turn), separate from any chat.

### Providers

**DeepSeek** — API key on this Runner, streamed through DeepSeek's Anthropic-compatible endpoint (`https://api.deepseek.com/anthropic`, `providers::anthropic`), the one that runs DeepSeek's web search on the server: every request declares Anthropic's `web_search_20250305` tool, and the model's searches stream back as `server_tool_use` and `web_search_tool_result` blocks. `TINYBOT_DEEPSEEK_MODEL` overrides the model; `TINYBOT_DEEPSEEK_BASE_URL` is the API root a proxy stands in for (the key check calls its `/models`, bots its `/anthropic`).

**Anthropic** — API key on this Runner, the Messages API (`providers::anthropic`): `claude-opus-5` by default with adaptive thinking, and Anthropic's `web_search_20260209` and `web_fetch_20260209` tools on every request (the basic variants and no thinking parameter for Haiku 4.5 and the 4.5 generation). `TINYBOT_ANTHROPIC_MODEL` and `TINYBOT_ANTHROPIC_BASE_URL` override the model and endpoint.

Both stream the same way: text, thinking (with its `signature`), `tool_use` blocks, and the server tools' own blocks. A server tool call becomes a `ServerToolStart` / `ServerToolEnd` pair for the Runner's activity rows, and the raw `server_tool_use` and `*_tool_result` blocks stay in the assistant message as `ServerBlock` parts, so when the same turn continues after a function call the model gets its searches, their results, and its sealed thinking back verbatim. A later turn's context is rebuilt from the chat and carries none of it.

**ChatGPT** — subscription OAuth on this Runner (`providers::chatgpt`, isolated): authorization code with PKCE, the localhost:1455 callback the Codex CLI uses, tokens refreshed by the adapter, and the Codex responses backend for streaming. `TINYBOT_CHATGPT_MODEL` overrides the model. Every request carries the backend's built-in `web_search` tool, which searches and reads pages server-side; its `web_search_call` items stream back as `ServerToolStart` / `ServerToolEnd` events (`web_search` with the query, `web_fetch` with the URL for an `open_page`).

Server tool events from any provider become tool rows on the Runner, so the status line reads "Searching the web…" or "Reading the web…" while one runs; `build_context` never replays those rows to the model.

The encrypted bot profile carries `provider` as a label; the Runner resolves it against its own credentials at turn time.

## macOS app

SPM `Tinybot.app`, AppKit.

The app starts the bundled `tinybot` (Contents/MacOS/tinybot; `TINYBOT_CLI` overrides, PATH is the fallback) as `tinybot serve --port <port>`, logs it to `~/Library/Logs/Tinybot/cli.log`, and restarts it if it exits. If something already listens on the port, that instance is used.

First run: the CLI answers `hello` with `has_identity: false`, and the app shows onboarding: create (the CLI returns the phrase), restore (phrase → relay), or pair (paste the string from the identity Mac). After create, the user names the first bot (the CLI's default Chef, edited through `bots.update`), then connects a provider on this Mac (a DeepSeek or Anthropic key, or a ChatGPT sign-in; skippable). Restore and pair go straight to the provider step, since the roster syncs.

Chrome: split view, vibrancy, bubbles, `@` mentions. The app renders CLI events and applies its own edits optimistically; `TINYBOT_MOCK=1` runs the seeded demo instead. Keys stay in the CLI.

Composer: the `+` button opens a file panel; files dropped on the composer or pasted (file URLs, or an image with no text beside it, written to a temporary PNG) attach the same way. Chips above the text show a thumbnail or the file's name and size, each with a remove button; a message can be attachments alone. The app mints the attachment ids and hands the CLI paths (`chats.send { attachments: [{ id, path, name, mime, width, height }] }`), so the bubble it shows at once matches the message the CLI echoes back. In a bubble, images are thumbnails sized from the width and height in the message and files are cards; a click opens the file. An attachment from another Device is fetched through `files.path`, which pulls the `file` blob from the relay into `~/.tinybot/files/` and answers with the path; the bubble reloads when it lands. The Dictate button (`mic.fill`, the primary disc while the field is empty) starts Apple's speech recognizer (`Speech` + `AVAudioEngine`, `Dictation.swift`). While recording, the text gives way to Grok Bot's pill: a stop square, the elapsed time, and bars that follow the input level, with Send still beside it. The transcript accumulates and lands at the caret when the square is clicked; Send commits it and sends in one go; Escape discards it. The recognizer listens in the language of the keyboard input source in use (a Pinyin input method means zh-CN, whatever the system language), else the first of the system's preferred languages it supports (matched on language and region, so "zh-Hans-CN" finds "zh-CN"), or the language chosen in Settings › General or the button's right-click menu (`Preferences.dictationLanguage`). The bundle declares `NSMicrophoneUsageDescription` and `NSSpeechRecognitionUsageDescription`.

Working state, after Grok Bot: the CLI's `job.started` / `job.finished` events (and `running_turns` in the snapshot) name the chat and bot of every turn in flight, including a turn sent to another Runner. From them the app shows a breathing green dot on the bot's avatar in the sidebar, an "is working" row after the last message (the avatar alone in a DM, "Chef is working…" or the bot's latest tool as an activity such as "Running commands…", kept between calls so it does not flash, in a group), the Stop button, and "Chef stopped without replying" when a turn ends with nothing said. Replies arrive in chunks: the bubble appears with the first completed paragraph (or sentence, after 1.5 s) and grows by paragraphs, the way Grok Bot's server re-sends a message as it grows. Tool calls never appear in the transcript, as in Grok Bot: the CLI keeps them as `tool` messages so a later turn can rebuild its context, and the app renders none of them, leaving the "is working" row (with the running tool's activity) and whatever the bot says before and after its work. The one exception is a sent `message_bot`, shown as the "Messaged ◉ X" marker. Sidebar rows carry a blue unread dot, a preview without a "You:" prefix (a bot's name only in groups; "Messaged X" and "Message from X: …" for bot-to-bot messages), and a stamp that is the time today, "Yesterday", the weekday within a week, then the date. A transcript inserts "Today 4:13 AM" separators after fifteen minutes of silence; in a group a bot's name sits above its bubble with its avatar beside the bubble's bottom edge, and a DM shows neither.

## Protocols

### App ↔ CLI (local WS)

JSON on `ws://127.0.0.1:4862/ws`. Requests are `{ id, method, params }` and get `{ id, result }` or `{ id, error: { message } }`; events are `{ event, data }`.

App → CLI: `hello`, `bootstrap`, `identity.create`, `identity.restore`, `pair.start` / `pair.status` / `pair.cancel` / `pair.accept`, `config.set`, `bots.create` (`runner_id` may be another Device; it must be a Runner) / `bots.update`, `chats.create` / `chats.dm` / `chats.send` / `chats.stop` / `chats.delete` / `chats.rename` / `chats.pin` / `chats.add_bot` / `chats.remove_bot` / `chats.mark_read`, `providers.connect_deepseek` / `providers.connect_anthropic` / `providers.connect_chatgpt` / `providers.disconnect` (this Runner).

CLI → App: `snapshot`, `roster.changed`, `message.added` / `message.updated` / `message.removed`, `chat.removed`, `job.started` / `job.finished`, `relay.status`, `pair.completed`, `identity.changed`.

The app may choose ids (`bots.create.id`, `chats.create.id`, `chats.send.message_id`) so its optimistic rows match the CLI’s events.

### CLI ↔ relay

- Machine bearer for blobs and presence; identity signature for registering and attesting machines.
- PUT/GET blobs; body is ciphertext. `roster` is a whole-roster snapshot (latest wins); `chat` is one upsert or removal of a message; `machine` is a Device’s metadata; `key` is the DEK sealed to the content key.
- Jobs: `kind=job` with `recipient_machine_pubkey`, sealed to that machine’s box key, deleted by the Runner after the turn.
- Every Device keeps `last_seq` and an outbox; uploads retry until the relay accepts them.

```
App_B → CLI_B
CLI_B encrypts user message + job-for-A → relay
CLI_A fetches envelopes for A, decrypts, runs loop, encrypts reply
CLI_B fetches chat blobs, decrypts → App_B
```

## Repo layout

```
tinybot/
  ARCHITECTURE.md
  README.md
  Cargo.toml           # workspace
  crates/agent/        # tinybot-agent: loop, tools, providers (Anthropic Messages for DeepSeek and Anthropic, ChatGPT)
  crates/cli/          # tinybot: keys, local WS, jobs, relay sync
  crates/relay/        # tinybot-relay: axum + SQLite
  macos/               # AppKit SPM app; the build bundles the CLI
  mobile/              # Expo app for iOS and Android: a paired Device speaking the relay protocol itself
  web/                 # the site
  scripts/             # bun scripts: dev loop, bundle build
```

`bun run mobile` starts the Expo dev server; `bun run mobile:ios` / `mobile:android` build and run the dev client.

`bun run dev` rebuilds the CLI and the app on Rust or Swift changes and relaunches the app. `bun run build` produces a release bundle. `bun run relay` runs a local relay.

## Status

Done: crypto and blob protocol, relay, CLI (identity, pairing, restore, local WS, DeepSeek and Anthropic keys, ChatGPT OAuth adapter, server-side web search, agent loop, encrypt-before-upload, group chats, cross-Runner jobs and handoffs, stop), app wiring and the bundled CLI launcher.

Next: context compaction, steering mid-turn, keychain storage, relay blob GC.

The phone app (`mobile/`) pairs as a Device with `os` `ios`, `ipados`, or `android`; it is never a Runner and does not hold the master secret. The first Mac is the identity device.

## Open points

- Relay blob compaction / GC
- Model ids move: DeepSeek defaults to `deepseek-flash` (`deepseek-v4-pro` for reasoning), Anthropic to `claude-opus-5` (`claude-sonnet-5`, `claude-fable-5-1`, `claude-opus-4-8`, `claude-haiku-4-5` offered), ChatGPT sign-ins default to `gpt-5.6-terra` (`gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-luna`, `gpt-5.5` also accepted; `*-codex` ids are rejected for ChatGPT accounts). Each bot carries an optional `model` (New Bot sheet, and the DM inspector's "Runs with" section); `TINYBOT_DEEPSEEK_MODEL` / `TINYBOT_ANTHROPIC_MODEL` / `TINYBOT_CHATGPT_MODEL` override the defaults for bots without one
- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.
