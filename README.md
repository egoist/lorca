# Tinybot

Tinybot is a Grok bot alternative: a native macOS AppKit app and a Rust CLI. The CLI is a localhost websocket service and the agent loop; the app is the UI for that CLI and launches it from its own bundle.

You create bots, talk to them 1:1, or put them in a group chat. Bots can hand work to each other and orchestrate, in the same spirit as Grok Bot.

The UI is AppKit (SPM), built to feel like a Mac app: materials, density, keyboard, and motion.

Identity is a local key pair. Devices pair to each other. Traffic to the network is end-to-end encrypted; the website is a [Happy](https://happy.engineering/docs/security/)-style relay for ciphertext. Each machine or phone that pairs is a Device and records its OS (`macos`, `linux`, `windows`, `ios`, `ipados`, `android`). Desktop Devices are Runners: you can create a bot for any paired Runner, and that bot’s DeepSeek key or ChatGPT subscription lives on the assigned Runner. Phones and tablets are Devices, not Runners.

## Stack

- macOS app: AppKit, SPM (`macos/`)
- CLI: Rust websocket service (`crates/cli`)
  - agent loop after pi-agent-core (`crates/agent`), with Grok-style bot orchestration
  - identity, pairing, E2E (X25519/Ed25519 key pairs + XChaCha20-Poly1305 DEK); signed requests and opaque blobs to the relay
- relay: Rust, axum, SQLite (`crates/relay`)

## Run it

```bash
bun run dev          # builds the CLI and the app, launches the app, rebuilds on change
bun run relay        # a local relay on 127.0.0.1:8787 (set TINYBOT_RELAY_URL to use it)
cargo test           # agent loop, crypto, and SSE tests
bun run reset        # stop everything and wipe identity, credentials, chats, prefs (-y, --build, --relay)
```

## AI services

- API key: DeepSeek
- Subscription: ChatGPT

Architecture: [ARCHITECTURE.md](./ARCHITECTURE.md). Read it before implementing.

## Later

- A mobile app (a Device, not a Runner)

## Website

The landing page lives in `web/` (TanStack Start on a Cloudflare Worker, shadcn/ui).

```bash
bun run web          # dev server on http://localhost:3000
bun run web:deploy   # build and wrangler deploy
```
