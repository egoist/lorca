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
6. **Provider credentials live on the Runner the bot is assigned to.** You can create a bot for any paired Runner. DeepSeek keys and ChatGPT tokens stay on that assigned machine.
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
5. B syncs the roster and chats and shows up in the Device list. If B is a Runner, connecting DeepSeek or ChatGPT on B happens on B.

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
| Job                | Any paired Device may create; the assigned Runner runs it | Sealed envelope to that Runner’s machine box key; deleted once run |

Creating a bot for Runner B from Device A: A writes an encrypted bot profile into the roster (paired Devices can read it) and pins B’s machine id. Bot create rejects a target whose `os` is not desktop. Turns are job envelopes addressed to B. B decrypts the job, runs the loop with B’s provider credentials, and uploads encrypted replies.

If B is offline or still connecting a provider, the envelope waits on the relay until B fetches it. The UI infers that from decrypted roster state.

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

`kind` is `roster` | `chat` | `job` | `machine` | `key`. Ciphertext is bytes; the nonce sits inside it. `seq` increases per identity. A Device’s `name` and `os` are inside its `machine` blob, not columns.

Blob API: `PUT /v1/blobs` (client-chosen id, idempotent), `GET /v1/blobs?since=<seq>&kinds=&wait=25` (long-poll; returns blobs for the identity that are unaddressed or addressed to the caller’s machine), `DELETE /v1/blobs/{id}`, `GET /v1/machines` (presence).

Clients set `TINYBOT_RELAY_URL` or the relay URL in Settings › Advanced. Without a relay the CLI works on one Device alone.

A CLI lists blobs for its identity and envelopes for its machine public key (long-poll), decrypts, and emits events to the app on localhost.

## CLI (runtime)

`crates/cli`, binary `tinybot`. Local websocket for the app. Files under `~/.tinybot/` (override with `TINYBOT_HOME`), all mode 0600: `identity.json` (master secret, identity devices only), `machine.json` (machine secret, identity and content public keys, account DEK, `name`, `os`), `credentials.json` (this Runner’s providers), `settings.json` (relay URL), `state.json` (plaintext roster, chats, sync cursor, outbox).

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
    events → transcript messages (thinking → streaming → complete) → local app WS
           → completed messages encrypted with the account DEK → relay
    tool calls execute; message_bot terminates the batch
```

Turns in one chat run one at a time on a Runner. `chats.stop` cancels the running turn.

### Tools

Team tools (the CLI):

- `message_bot { bot, message }` — handoff, visible in the transcript as a handoff row. Creates a `handoff` Job for the target bot; the envelope goes to the **target bot’s Runner**. The calling bot’s turn ends.
- `list_teammates` — decrypted local roster with Runner and online state.
- `create_bot { name, tagline, instructions, provider?, workdir? }` — a new teammate on the caller’s Runner with its own DM; in a group chat it joins that chat. This is how a lead bot builds its team.

Coding tools (`tinybot_agent::tools`, ports of pi’s built-ins, same schemas and truncation rules: 2000 lines / 50KB, whichever first):

- `read { path, offset?, limit? }`, `write { path, content }`, `edit { path, edits: [{ oldText, newText }] }`
- `bash { command, timeout? }` — tail-truncated, full output to a temp file, process group killed on stop
- `grep { pattern, path?, glob?, ignoreCase?, literal?, context?, limit? }`, `find { pattern, path?, limit? }`, `ls { path?, limit? }` — respect `.gitignore` through the `ignore` walker, no `rg`/`fd` needed

Every bot has a working directory on its Runner (`workdir`, default `~/.tinybot/workspaces/<bot id>`, so a rename never moves files); relative paths resolve there.

### The lead bot

A new identity starts with one bot, **Chef**, a chief of staff on the first Mac: an ordinary bot with a default profile whose instructions are to learn the user’s work, propose a small team of one-job bots, create them with `create_bot`, and route work with `message_bot`. Nothing about it is privileged; rename or delete it like any bot.

A chat has a `kind`. A DM is one bot and never gains or loses members; there is one DM per bot. A group holds one to six bots and can add or remove them after creation. Group chats: `@BotName` / `@everyone`; otherwise one owner. Orchestration is bots messaging bots.

### Providers

**DeepSeek** — API key on this Runner, OpenAI-compatible streaming Completions (`providers::openai_compat`). `TINYBOT_DEEPSEEK_MODEL` and `TINYBOT_DEEPSEEK_BASE_URL` override the model and endpoint.

**ChatGPT** — subscription OAuth on this Runner (`providers::chatgpt`, isolated): authorization code with PKCE, the localhost:1455 callback the Codex CLI uses, tokens refreshed by the adapter, and the Codex responses backend for streaming. `TINYBOT_CHATGPT_MODEL` overrides the model.

The encrypted bot profile carries `provider` as a label; the Runner resolves it against its own credentials at turn time.

## macOS app

SPM `Tinybot.app`, AppKit.

The app starts the bundled `tinybot` (Contents/MacOS/tinybot; `TINYBOT_CLI` overrides, PATH is the fallback) as `tinybot serve --port <port>`, logs it to `~/Library/Logs/Tinybot/cli.log`, and restarts it if it exits. If something already listens on the port, that instance is used.

First run: the CLI answers `hello` with `has_identity: false`, and the app shows onboarding: create (the CLI returns the phrase), restore (phrase → relay), or pair (paste the string from the identity Mac). After create, the user names the first bot (the CLI's default Chef, edited through `bots.update`), then connects a provider on this Mac (DeepSeek key or ChatGPT sign-in, skippable). Restore and pair go straight to the provider step, since the roster syncs.

Chrome: split view, vibrancy, bubbles, `@` mentions. The app renders CLI events and applies its own edits optimistically; `TINYBOT_MOCK=1` runs the seeded demo instead. Keys stay in the CLI.

## Protocols

### App ↔ CLI (local WS)

JSON on `ws://127.0.0.1:4862/ws`. Requests are `{ id, method, params }` and get `{ id, result }` or `{ id, error: { message } }`; events are `{ event, data }`.

App → CLI: `hello`, `bootstrap`, `identity.create`, `identity.restore`, `pair.start` / `pair.status` / `pair.cancel` / `pair.accept`, `config.set`, `bots.create` (`runner_id` may be another Device; it must be a Runner) / `bots.update`, `chats.create` / `chats.dm` / `chats.send` / `chats.stop` / `chats.delete` / `chats.rename` / `chats.pin` / `chats.add_bot` / `chats.remove_bot` / `chats.mark_read`, `providers.connect_deepseek` / `providers.connect_chatgpt` / `providers.disconnect` (this Runner).

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
  crates/agent/        # tinybot-agent: loop, tools, providers (DeepSeek, ChatGPT)
  crates/cli/          # tinybot: keys, local WS, jobs, relay sync
  crates/relay/        # tinybot-relay: axum + SQLite
  macos/               # AppKit SPM app; the build bundles the CLI
  scripts/             # bun scripts: dev loop, bundle build
```

`bun run dev` rebuilds the CLI and the app on Rust or Swift changes and relaunches the app. `bun run build` produces a release bundle. `bun run relay` runs a local relay.

## Status

Done: crypto and blob protocol, relay, CLI (identity, pairing, restore, local WS, DeepSeek, ChatGPT OAuth adapter, agent loop, encrypt-before-upload, group chats, cross-Runner jobs and handoffs, stop), app wiring and the bundled CLI launcher.

Next: context compaction, steering mid-turn, keychain storage, relay blob GC, streaming partial messages to other Devices.

Mobile can later hold the master secret the way Happy’s phone does. A phone or tablet pairs as a Device with `os` `ios`, `ipados`, or `android`; it is never a Runner. Until then the first Mac is the identity device.

## Open points

- Relay blob compaction / GC
- Model ids move: DeepSeek defaults to `deepseek-flash` (`deepseek-v4-pro` for reasoning), ChatGPT sign-ins default to `gpt-5.6-terra` (`gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-luna`, `gpt-5.5` also accepted; `*-codex` ids are rejected for ChatGPT accounts). Each bot carries an optional `model` (New Bot sheet, and the DM inspector's "Runs with" section); `TINYBOT_DEEPSEEK_MODEL` / `TINYBOT_CHATGPT_MODEL` override the defaults for bots without one
- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.
