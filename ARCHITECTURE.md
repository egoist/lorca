# Tinybot Architecture

Source of truth for how Tinybot is built. Read this before writing code.

Tinybot is a Grok Bot alternative: persistent named bots, 1:1 chats, group chats, handoff, and orchestration. Work runs on **Computers you own**. The macOS app is the UI for the local Rust CLI.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM): system materials, SF Symbols, Auto Layout, keyboard, accessibility.
3. **The CLI owns the agent loop:** inference, tools, streaming, cancellation, orchestration.
4. **The relay is zero-knowledge:** opaque blobs and public keys. Auth is a signature challenge.
5. **Provider credentials live on the Computer the bot is assigned to.** You can create a bot for any paired Computer. DeepSeek keys and ChatGPT tokens stay on that assigned machine.
6. **A bot runs on one Computer:** that Computer’s CLI.

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
                                                 │  Relay (Cloudflare       │
                                                 │  Worker + D1)            │
                                                 │  public keys + blobs     │
                                                 └──────────────────────────┘
```

- **App:** native chat UI. Create or restore identity, pair, through the CLI.
- **CLI:** identity and machine keys, local websocket, encrypt/decrypt, agent loop, this Computer’s provider credentials, sync with the relay.
- **Relay / website:** marketing site and store-and-forward API. TanStack Start + Query, Tailwind, shadcn, Cloudflare Worker, Drizzle, D1.

If the CLI is down, the app shows a native empty state to start it.

## Identity

An identity is keys you hold.

Mac-first, after Happy’s layering. Until a phone exists, the first Computer is the identity device.

| Layer | What | Where |
| --- | --- | --- |
| Master secret (32 bytes) | Root. Backup as a base32 phrase. | Keychain / `~/.tinybot/`. Stays on the identity device. |
| Content keypair | Derived from master (HKDF). Secret unwraps DEKs. Public key wraps DEKs. | Secret on devices that have the master. Public key may sit on the relay. |
| Identity signing key | Ed25519 (or NaCl equivalent). | Private local. Public key is the identity id (`hash(pubkey)`). |
| Account DEK | Random AES-256-GCM (or XChaCha20-Poly1305). Encrypts roster, bots, chats, messages. | Made locally. On the relay **wrapped** to the content public key and each paired machine public key. |
| Machine keypair | One per Computer. | Private local. Public key on the relay. |
| Machine DEK | Encrypts that Computer’s metadata (name, OS, presence blob). | Wrapped to the content public key; relay stores ciphertext. |
| Chat/job envelopes | Symmetric DEK or sealed box to a machine public key. | Relay stores ciphertext. |
| Ephemeral pairing key | One handshake. | Devices; discarded after pairing. |

Recovery: restore the master secret from the backup phrase → re-derive content keys → unwrap DEKs from the relay. The backup phrase is the identity.

### Pairing a Computer

1. Computer A (has the identity) shows a QR / pairing string: relay URL, identity public key, ephemeral public key, short-lived pairing nonce.
2. Computer B’s CLI scans or pastes it. The two devices run a NaCl Box (Curve25519) handshake. The relay can carry that ciphertext.
3. A wraps the **account DEK** (roster/chats) to B’s machine public key. Provider credentials stay on each Computer.
4. B registers its machine public key on the relay, attested by the identity.
5. B decrypts account content and shows up in the Computer list. Connecting DeepSeek or ChatGPT on B happens on B.

App ↔ CLI on one machine uses `127.0.0.1`; those keys are already local.

### Relay surface

The relay stores:

- Identity public key and machine public keys
- Blob ids, sequence numbers, timestamps, size
- Recipient machine public key on an envelope (so a Computer can fetch its jobs)
- Last-seen of a machine public key

Nicknames, bot profiles, and chat text live inside encrypted blobs.

## Domain model

Plaintext lives **on devices**:

```
Identity 1──* Computer 1──* Bot
Identity 1──* Chat
Chat      *──* Bot          (kind dm: exactly 1 bot, fixed · kind group: 1–6 bots, members change)
Chat      1──* Message
Bot       1──* Job          (a turn on the bot's Computer)
```

| Entity | Device | Relay |
| --- | --- | --- |
| Identity | Master + content + signing keys | Public key |
| Computer | Machine keypair, local provider creds | Machine public key + encrypted metadata blob |
| Bot | Decrypted profile | Inside encrypted roster blobs |
| ProviderCredential | Assigned Computer’s keychain | — |
| Chat / Message | Account/chat DEK | Encrypted blobs |
| Job | Any paired CLI may create; the assigned Computer runs it | Encrypted envelope to that Computer’s machine public key |

Creating a bot for Computer B from Computer A: A writes an encrypted bot profile into the roster (paired devices can read it) and pins B’s machine id. Turns are job envelopes addressed to B. B decrypts the job, runs the loop with B’s provider credentials, and uploads encrypted replies.

If B is offline or still connecting a provider, the envelope waits on the relay until B fetches it. The UI infers that from decrypted roster state.

## Credential locality

- **Provider setup** on that Computer (keychain or `~/.tinybot/credentials`, mode `0600`).
- **Bot create** may target any paired Computer. The relay payload is ciphertext of the profile.

## Relay / website

TanStack Start + TanStack Query, Tailwind, shadcn/ui, Cloudflare Workers, Drizzle ORM, Drizzle Kit, D1.

Auth:

1. Client sends identity public key.
2. Relay returns a nonce.
3. Client signs the nonce with the identity signing key.
4. Relay issues a short-lived bearer bound to that pubkey.

Drizzle, for example:

- `identities(pubkey, created_at)`
- `machines(identity_pubkey, machine_pubkey, last_seen)`
- `blobs(id, identity_pubkey, kind, recipient_machine_pubkey nullable, seq, ciphertext, nonce, created_at)`

`kind` is `roster` | `chat` | `job` | `machine`. Ciphertext is bytes.

The site is marketing, docs, and pairing. Clients set `TINYBOT_RELAY_URL` (self-host included).

A CLI lists blobs for its identity and envelopes for its machine public key (poll or long-poll), decrypts, and emits events to the app on localhost.

## CLI (runtime)

Rust. Tokio. Local websocket for the app. HTTPS to the relay. libsodium-compatible NaCl box + AEAD.

Commands:

- `tinybot serve` — default; the app connects here
- `tinybot identity new` / `identity restore`
- `tinybot pair` — show QR or consume a pairing string
- `tinybot status` / `doctor`

Bind: `127.0.0.1:4862`. Relay: `TINYBOT_RELAY_URL`.

### Agent loop

References: vercel-labs/fx, OpenCode, pi-agent-core.

```
on decrypted Job:
  loop:
    stream provider(messages, tools)   # this Computer's creds
    emit events to the local app WS
    encrypt + upload blobs to the relay
    if tool_calls: execute; continue
    else: break
```

Encrypt transcripts before upload.

### Tools

- `message_bot` — handoff, visible in the transcript. Job envelope goes to the **target bot’s Computer**.
- `list_teammates` — decrypted local roster.

A chat has a `kind`. A DM is one bot and never gains or loses members; there is one DM per bot. A group holds one to six bots and can add or remove them after creation. Group chats: `@BotName` / `@everyone`; otherwise one owner. Orchestration is bots messaging bots.

### Providers

**DeepSeek** — API key on this Computer, OpenAI-compatible Completions.

**ChatGPT** — subscription OAuth on this Computer. Isolate that adapter.

The encrypted bot profile may include `provider` as a label.

## macOS app

SPM `Tinybot.app`, AppKit.

First run: create identity and show the backup phrase, or restore. Other Macs: pair.

Chrome: split view, vibrancy, bubbles, `@` mentions. The app renders CLI events. Keys stay in the CLI / keychain.

## Protocols

### App ↔ CLI (local WS)

JSON on localhost.

App → CLI: `hello`, `identity.create`, `identity.restore`, `pair.start`, `pair.accept`, `bootstrap`, `bots.create` (`computer_id` may be another machine), `chats.send`, `providers.connect_deepseek`, `providers.connect_chatgpt` (this Computer), …

CLI → App: `snapshot`, `message.*`, `tool.*`, `job.*`.

### CLI ↔ relay

- Sign mutating requests.
- PUT/GET blobs; body is ciphertext.
- Jobs: `kind=job` with `recipient_machine_pubkey`.

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
  web/                 # marketing site + relay Worker
  cli/                 # Rust crate (keys, loop, local WS)
  macos/               # AppKit SPM app
```

## Implementation order

1. **Crypto + blob protocol** (identity, pair, wrap DEKs, relay blob schema).
2. **Relay Worker** (challenge-response, Drizzle blobs).
3. **CLI:** identity, pair, local WS, DeepSeek, agent loop, encrypt-before-upload.
4. **App:** identity/pair chrome, sidebar, DM chat, streaming bubbles.
5. **ChatGPT adapter** (local OAuth).
6. **Group chats + jobs addressed to another Computer.**
7. Compaction, stop/steering, keychain, backup phrase UX.

Mobile can later hold the master secret the way Happy’s phone does. Until then the first Mac is that device.

## Open points

- AEAD: AES-256-GCM vs XChaCha20-Poly1305; TweetNaCl vs dalek + `crypto_box`
- Pairing wraps the account DEK to each machine public key (Happy-style)
- Relay blob compaction / GC
- DeepSeek model id and ChatGPT slug at ship time
- Bundled CLI vs `PATH`

When those are chosen, update this file.
