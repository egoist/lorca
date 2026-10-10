# Lorca Architecture

Source of truth for how Lorca is built. Read this before writing code, then the docs in [Subjects](#subjects) for the parts you change.

Lorca is a Grok Bot alternative: persistent named bots, 1:1 chats, group chats, handoff, and orchestration. Work runs on **Devices you own**. The Rust CLI runs on macOS, Linux, and Windows. The macOS app, and the app for Windows and Linux, are UIs for the local CLI, which each bundles and launches.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The app ships the CLI binary inside its bundle and starts `lorca serve` itself, unless one already answers on the port. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM) on macOS: system materials, SF Symbols, Auto Layout, keyboard, accessibility. On Windows and Linux it is a MyGo app in Go, drawn with MyGo's native UI toolkit, that follows the macOS app screen for screen.
3. **The CLI owns the agent loop:** inference, tools, streaming, cancellation, orchestration.
4. **The relay is zero-knowledge:** opaque blobs and public keys. Auth is a signature challenge.
5. **Every Device records its `os`.** A Device with a desktop `os` (`macos`, `linux`, `windows`) is a **Runner**. Phones and tablets (`ios`, `ipados`, `android`) are Devices, never Runners.
6. **Provider credentials belong to the account.** API keys, ChatGPT and Grok tokens, and custom providers (any server that speaks OpenAI's or Anthropic's API, or a decision API) are connected once, on any Device, and reach every paired Device as a `credentials` blob encrypted with the account DEK. A bot runs with them on whichever Runner it is assigned to.
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
Bot      1──* Channel      (a Telegram or Slack account it listens on; each thread is a Chat)
Device   1──* Plugin       (an MCP server installed on a Runner or in its mcp.json, per bot Access)
Bot      1──* Job          (a turn on the bot's Runner)
Bot      1──* Handoff      (a durable delegated request with attempts and result reports)
Identity 1──* Task         (durable work; an owning Bot, assigned Runner, and linked Chats)
```

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

Bots carry a name, description, provider/model settings, and an SF Symbol or encrypted avatar attachment. The apps edit these profiles through the local CLI ([Bot profiles](docs/architecture/bots.md#bot-profiles)).

A bot made from one Device runs on its Runner, which can be offline: [A bot on another Runner](docs/architecture/bots.md#a-bot-on-another-runner).

## Subjects

One doc per subject under `docs/architecture/`, each short enough to read in one pass. `bun run check:docs` holds this file to 16 KiB and each subject to 24 KiB, and checks that every subject is listed here and that links and their anchors resolve.

| Doc | Read it for |
| --- | --- |
| [Identity](docs/architecture/identity.md) | Key pairs and the identity device, pairing and unpairing, Devices and Runners, what the relay sees, the account's provider credentials |
| [Relay](docs/architecture/relay.md) | `crates/relay`: storage on SQLite or Postgres, files, housekeeping, quotas, metrics, rate limits, auth, tables and migrations, the blob, sync socket, and push APIs, deploys |
| [Protocols](docs/architecture/protocols.md) | The app ↔ CLI websocket and the CLI ↔ relay requests and blobs |
| [CLI (runtime)](docs/architecture/runtime.md) | The `lorca` binary and its data directory, installing it, its signed self-updates and `lorca service`, the agent loop and a turn on a Runner, notifications |
| [Bot permissions](docs/architecture/bot-permissions.md) | A bot's Access to plugins, tools, files, and shell; where the CLI checks it; access requests |
| [Tools](docs/architecture/tools.md) | Team, memory, and coding tools, Auto-review |
| [Handoffs](docs/architecture/handoffs.md) | Durable delegated contracts, expected outputs, result evidence and return routing, offline delivery and restart recovery |
| [Review queue](docs/architecture/review-queue.md) | Encrypted editable proposals, version-bound approval, Runner execution and outcomes |
| [Message drafts](docs/architecture/drafts.md) | Emails and Slack messages a bot writes in a chat as drafts the user edits, sends, or discards; sending directly; edits as feedback |
| [Outputs and evidence](docs/architecture/outputs.md) | Bot-generated files and document links, immutable versions, task evidence references, encrypted transport and native previews |
| [Stop](docs/architecture/stopping.md) | What Stop ends: the turn's model call and tool calls, a call that never returns, commands and browser calls, the work it handed off on any Runner; what keeps running |
| [Terminal sessions](docs/architecture/terminal-sessions.md) | A bot's commands in terminals of their own: when a call returns, background commands, the command's card, answering and stopping, Running tasks |
| [Secret requests](docs/architecture/secrets.md) | A password, key, or code a bot asks for on a card: sealed to its Runner, kept there, filled into Browser on its site or a command's environment by name, kept out of results and chats; managing a Runner's secrets |
| [Browser sessions](docs/architecture/browser-sessions.md) | A bot's browser profiles on its Runner: their sign-ins, opening one in a window, taking the browser over and handing it back, screenshots in the chat, what works from another Device, the Profiles section |
| [Codemode and Plugins](docs/architecture/plugins.md) | Scripts that call plugin tools, MCP plugins and their installs, sign-in, plugin calls at turn time |
| [Integrations and named accounts](docs/architecture/integrations.md) | Slack, Gmail, Calendar, and Drive; stable account instances, OAuth on the Runner, account selection, access recovery, and source links |
| [Marketplace](docs/architecture/marketplace.md) | The index of plugins and bot templates and how lorca.app keeps it current on every Device, bots added from a template, the marketplace sheet |
| [Workflow onboarding](docs/architecture/workflows.md) | Workflows from the marketplace: packs, a setup on a Runner and its accounts, the sample before the schedule, how setups sync, the workflow page in each app |
| [Bot templates](docs/architecture/templates.md) | A bot shared as a link or a file and a new bot made from one: what a template holds, redaction and flags, encrypted links and their keys, the Runner's own connections, the apps' sheets |
| [MCP servers](docs/architecture/mcp-servers.md) | The user's own MCP servers in a Runner's `mcp.json`: the file and other apps' spellings, sign-in when a server asks, the `mcp.*` methods and `lorca mcp`, the apps' MCP Servers section and server sheet |
| [Bots and Memory](docs/architecture/bots.md) | The lead bot, DMs and groups, who answers, handoffs between bots, a bot's memory |
| [Routines](docs/architecture/routines.md) | A bot's scheduled tasks: schedules and their timezones, runs and read-only checks, missed runs, health and recovery, and the apps' routine sheet and service row |
| [One-time routines, watches, and calendar events](docs/architecture/routine-triggers.md) | Routines that run once at a date and time, watch one pull request until it merges or closes, or run around a Calendar account's events; how the Runner reads GitHub and Calendar for them |
| [Event triggers](docs/architecture/event-triggers.md) | Runner event subscriptions, gateways that sign and seal a service's events, a routine's webhook, why the relay holds no webhook inbox, the encrypted inbox, ordering and recovery |
| [Channels](docs/architecture/channels.md) | A bot listening on Telegram and Slack: the accounts and their builtin servers, filters, the Runner's readers, conversations and what people write there, replies, the feedback collector |
| [Durable tasks](docs/architecture/tasks.md) | Work that spans turns: owner, revisions, runs and recovery, evidence, the apps' Tasks section |
| [Coordinator attention](docs/architecture/attention.md) | Consolidated reviews, blockers, commitments and changes, coordinator briefs, deduplication, encrypted records and notification preferences |
| [Shared project context](docs/architecture/project-context.md) | Group briefs, goals, constraints, decisions, source freshness and corrections, encrypted reference assets, bounded bot discovery |
| [Playbooks](docs/architecture/playbooks.md) | Skills users write for a bot or group, saving one from a chat, revisions, discovery, export |
| [Workflow feedback](docs/architecture/feedback.md) | Feedback on bots' work, suggested changes to routines and skills, versions and undo, exclusions |
| [Providers](docs/architecture/providers.md) | Each model provider and its sign-in, custom providers, thinking levels, the model catalog and cost, compaction, retries |
| [Budgets and connector limits](docs/architecture/budgets.md) | Limits on turns, tasks, and routines, stopping and resuming, price labels, shared plugin call limits |
| [macOS app](docs/architecture/macos-app.md) | The AppKit app: launching the CLI, windows and onboarding, settings, updates, the command palette, sidebar and inspector |
| [macOS chat](docs/architecture/macos-chat.md) | Transcript, composer, output previews, working and read state in AppKit |
| [Windows and Linux app](docs/architecture/desktop-app.md) | The MyGo app: its model, host, and native views, title bar, commands, updates, development and builds |
| [Phone app](docs/architecture/phone-app.md) | The Expo app over the Rust core: the native module, pairing, relay status, attachments, dictation, notifications, turns |
| [Website](docs/architecture/website.md) | `web/`: the site, its docs, and the install scripts it serves |
| [Languages](docs/architecture/languages.md) | English and Simplified Chinese in each app, and what the CLI words |
| [Repo layout and development](docs/architecture/development.md) | Where each crate and app lives, the Mac, phone, and Android development loops, release bundles and their SDK restamp, a local relay |

## Status

Done: crypto and blob protocol, relay, CLI (identity, pairing, restore, local WS, API-key and subscription providers, server-side web search, agent loop, encrypt-before-upload, group chats, cross-Runner jobs and handoffs, steering and stop, routines, limits on turns and routines with resuming, shared plugin call limits, plugins over MCP with a marketplace, channels on Telegram and Slack, the user's own servers in `mcp.json`, and permission cards, encrypted pushes for replies, failures, and pending confirmations, signed self-updates of a CLI without an app, updated from any Device, and `lorca service`), app wiring and the bundled CLI launcher.

Next: keychain storage.

The phone app (`mobile/`) pairs as a Device with `os` `ios`, `ipados`, or `android`; it is never a Runner and does not hold the master secret. The Device that creates or restores the identity holds the master secret.

## Open points

- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.
