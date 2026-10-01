# Lorca Architecture

Source of truth for how Lorca is built. Read this before writing code, then the docs in [Subjects](#subjects) for the parts you change.

Lorca is a Grok Bot alternative: persistent named bots, 1:1 chats, group chats, handoff, and orchestration. Work runs on **Devices you own**. The Rust CLI runs on macOS, Linux, and Windows. The macOS app, and the app for Windows and Linux, are UIs for the local CLI, which each bundles and launches.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The app ships the CLI binary inside its bundle and starts `lorca serve` itself, unless one already answers on the port. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM) on macOS: system materials, SF Symbols, Auto Layout, keyboard, accessibility. On Windows and Linux it is a MyGo app, Go and the system webview, that follows the macOS app screen for screen.
3. **The CLI owns the agent loop:** inference, tools, streaming, cancellation, orchestration.
4. **The relay is zero-knowledge:** opaque blobs and public keys. Auth is a signature challenge.
5. **Every Device records its `os`.** A Device with a desktop `os` (`macos`, `linux`, `windows`) is a **Runner**. Phones and tablets (`ios`, `ipados`, `android`) are Devices, never Runners.
6. **Provider credentials belong to the account.** API keys and ChatGPT and Grok tokens are connected once, on any Device, and reach every paired Device as a `credentials` blob encrypted with the account DEK. A bot runs with them on whichever Runner it is assigned to.
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
                                                 │  SQLite or Postgres)     │
                                                 │  public keys + blobs     │
                                                 └──────────────────────────┘
```

- **App:** native chat UI. Create or restore identity, pair, through the CLI. Launches the bundled CLI as a child process and restarts it if it exits.
- **CLI:** identity and machine keys, local websocket, encrypt/decrypt, agent loop, the account’s provider credentials, sync with the relay.
- **Relay:** store-and-forward API. Rust, axum, SQLite or Postgres (`crates/relay`). Self-host it anywhere; clients point `LORCA_RELAY_URL` at it.

Production installs are named **Lorca** and use `app.lorca`; development installs are named **Lorca Dev** and use `app.lorca.dev`. They are separate applications on macOS, Windows, Linux, iOS, and Android. The production CLI stores its account under `~/.lorca` and listens on `4862`, while the desktop-app development build sets `LORCA_HOME=~/.lorca-dev` and listens on `4863`. On phones the distinct application ids give each build its own OS sandbox, and the core uses `lorca/core` or `lorca-dev/core` inside that sandbox. Their iOS keychain groups are `group.app.lorca` and `group.app.lorca.dev`.

If the CLI is down, the app shows a native empty state with the launcher’s status and the manual `lorca serve` command.

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
| Device             | Machine keypair, `os`                                     | Machine public key + encrypted metadata blob           |
| Bot                | Decrypted profile                                         | Inside encrypted roster blobs                          |
| Routine            | Name, schedule, prompt, state                             | Inside encrypted roster blobs                          |
| Plugin             | Manifest, variables, secrets, tokens on the Runner        | Id and state inside the Runner's encrypted machine blob |
| ProviderCredential | `credentials.json` on every Device                        | Inside the encrypted `credentials` blob                |
| Chat / Message     | Account/chat DEK                                          | Encrypted blobs                                        |
| Job                | Any paired Device may create; the assigned Runner runs it | Sealed envelope to that Runner’s machine box key; deleted once run. A hard Stop sends `job_cancel` to the Device running it; the Runner seals how the turn ended (`job_result`) to the requesting Device, and lists the turn and what it is doing in its `machine` blob for every Device |

A bot's look is an SF Symbol (`symbol_name`) on an accent gradient (`accent`), or a profile image of the user's own: `avatar` is an attachment record whose bytes travel as an encrypted `file` blob, the same way a message attachment does, and every Device shows the image in place of the symbol once it has fetched it. Clicking a bot's avatar in the macOS inspector opens the Look sheet. The macOS Profile card keeps Name trailing-aligned and opens Description in its own editing sheet. On the phone, tapping the avatar in Details slides the Look screen in inside the same form sheet (`app/chat-info` has its own stack); Name is trailing-aligned there too, and the Description row pushes a full editor. Details also has native Provider, Model, and Thinking menus under Runs with; a provider change clears the other two to that provider's defaults, and Thinking offers only the levels the model takes (see [Providers](docs/architecture/providers.md)). `bots.update` carries all of these changes, including `symbol_name`, `accent`, and `avatar` (a `{ path, … }` file to store and upload, or `null` to remove).

Creating a bot for Runner B from Device A: A writes an encrypted bot profile into the roster (paired Devices can read it) and pins B’s machine id. Bot create rejects a target whose `os` is not desktop. Turns are job envelopes addressed to B. B decrypts the job, runs the loop with the account’s provider credentials, and uploads encrypted replies.

If B is offline, the envelope waits on the relay until B fetches it. The UI infers that from decrypted roster state. A turn for a provider the account has not connected ends with a notice in the chat that says to connect it in Settings.

## Subjects

One doc per subject under `docs/architecture/`, each short enough to read in one pass. `bun run check:docs` holds this file to 16 KiB and each subject to 24 KiB, and checks that every subject is listed here and that links and their anchors resolve.

| Doc | Read it for |
| --- | --- |
| [Identity](docs/architecture/identity.md) | Key pairs and the identity device, pairing and unpairing, Devices and Runners, what the relay sees, the account's provider credentials |
| [Relay](docs/architecture/relay.md) | `crates/relay`: storage on SQLite or Postgres, files, housekeeping, quotas, metrics, rate limits, auth, tables and migrations, the blob, sync socket, and push APIs, deploys |
| [Protocols](docs/architecture/protocols.md) | The app ↔ CLI websocket and the CLI ↔ relay requests and blobs |
| [CLI (runtime)](docs/architecture/runtime.md) | The `lorca` binary and its data directory, installing it, the agent loop and a turn on a Runner, notifications |
| [Tools](docs/architecture/tools.md) | Team, memory, and coding tools, terminal sessions and command cards, running tasks, Auto-review |
| [Codemode and Plugins](docs/architecture/plugins.md) | Scripts that call plugin tools, MCP plugins and their installs, the marketplace and bot templates, sign-in, plugin calls at turn time |
| [Bots, Routines, and Memory](docs/architecture/bots.md) | The lead bot, DMs and groups, who answers, handoffs between bots, routines and their checks, a bot's memory |
| [Providers](docs/architecture/providers.md) | Each model provider and its sign-in, thinking levels, the model catalog and cost, compaction, retries |
| [macOS app](docs/architecture/macos-app.md) | The AppKit app: launching the CLI, windows and onboarding, settings, updates, the command palette, transcript, sidebar, inspector, composer, working state |
| [Windows and Linux app](docs/architecture/desktop-app.md) | The MyGo app: its Go side and Solid page, title bar, commands, updates, development and builds |
| [Phone app](docs/architecture/phone-app.md) | The Expo app over the Rust core: the native module, pairing, relay status, attachments, dictation, notifications, turns |
| [Website](docs/architecture/website.md) | `web/`: the site, its docs, and the install scripts it serves |
| [Languages](docs/architecture/languages.md) | English and Simplified Chinese in each app, and what the CLI words |

## Repo layout

```
lorca/
  ARCHITECTURE.md      # this overview and the list of subjects
  README.md
  docs/architecture/   # one doc per subject
  docs/agent/          # lorca-agent's own documentation
  Cargo.toml           # workspace
  crates/agent/        # lorca-agent: loop, tools, codemode (QuickJS), and Messages, Chat Completions, Responses, ChatGPT, and Grok providers
  crates/models/       # lorca-models: the model catalog (windows, thinking levels, rates), on every Device and in every app's pickers
  crates/provider-auth/ # OAuth token types and PKCE flows shared by every Device
  crates/cli/          # lorca: the Device core as a library (keys, relay sync, jobs, the JSON API) + runner and server features + the binary
  crates/mobile/       # lorca-mobile: the core for the phone over UniFFI
  crates/markdown/     # lorca-markdown: message Markdown as the blocks and spans every app renders (pulldown-cmark, and GitHub's autolinks for bare URLs and addresses), for the Mac and phone over UniFFI
  crates/relay/        # lorca-relay: axum + SQLite or Postgres, and its Dockerfile
  macos/               # AppKit SPM app; the build bundles the CLI
  desktop/             # the Windows and Linux app: MyGo (Go + system webview) with a Solid page; the build bundles the CLI
  mobile/              # Expo app for iOS and Android: a paired Device over the core (modules/lorca-core)
  web/                 # the site
  scripts/             # bun scripts: dev loop, bundle build, macOS release, the desktop app's dev loop and builds, string and doc checks
  .github/workflows/   # release-cli.yml and release-desktop.yml: release builds; test.yml: every app's and crate's tests on each pull request; docs.yml: the doc check
```

`bun run android` rebuilds the Rust core for Android, then builds and runs the Expo dev client on the Android emulator. `cd mobile && bun run core` rebuilds the Rust core for both phone platforms; `bun run mobile:dev` is the iOS development loop described below.

`bun run dev` rebuilds the CLI and Lorca Dev on Rust or Swift changes (the generated markdown bindings under `macos/Sources/LorcaMarkdown` are left out of the watch, and rewritten only when they differ) and relaunches the app through `open`, so the app is its own responsible process for TCC: a binary spawned from the terminal is charged to the terminal app, whose Info.plist decides whether a microphone or speech request aborts. `bun run build` produces the Lorca release bundle. The bundle step restamps the app binary's SDK version (`stampSDK` in `scripts/app.ts`, through `vtool`): the Swift Build engine writes the deployment target (14.0) there, and AppKit gives a binary stamped below the macOS 26 SDK its older look, with a flat sidebar and an opaque titlebar strip. `bun run relay` runs a local relay. `bun run mobile:dev` (`scripts/mobile.ts`) is the Lorca Dev phone loop on the iOS Simulator, or on `--device <name or udid>`: it fingerprints the crates the phone links, the prebuild inputs (Expo config, assets, `package.json`, plugins, targets), the pod inputs with the checkout's path, and the native module sources (stamps in `mobile/.expo/dev-stamps.json`), rebuilds what is stale (`bun run core ios`, a clean `expo prebuild`, `pod install`, `expo run:ios`), starts Metro, and opens the dev client on it. A Rust save while it runs rebuilds the core and installs the app again. The Mac and phone loops leave production Lorca processes alone, so all builds run side by side.

## Status

Done: crypto and blob protocol, relay, CLI (identity, pairing, restore, local WS, API-key and subscription providers, server-side web search, agent loop, encrypt-before-upload, group chats, cross-Runner jobs and handoffs, steering and stop, routines, plugins over MCP with a marketplace and permission cards, encrypted pushes for replies, failures, and pending confirmations), app wiring and the bundled CLI launcher.

Next: keychain storage, a cost budget per chat.

The phone app (`mobile/`) pairs as a Device with `os` `ios`, `ipados`, or `android`; it is never a Runner and does not hold the master secret. The Device that creates or restores the identity holds the master secret.

## Open points

- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.
