# Lorca

Lorca is a Grok bot alternative: a Rust CLI for macOS, Linux, and Windows, a desktop app for the three, and phone apps. The CLI is a localhost websocket service and the agent loop; the desktop app is a UI for that CLI and launches the bundled binary.

You create bots, talk to them 1:1, or put them in a group chat. Bots can hand work to each other and orchestrate, in the same spirit as Grok Bot.

The desktop app is Go drawn with MyGo's native UI, built to feel native on each system: materials, density, keyboard, and motion.

Identity is a local key pair. Devices pair to each other. Traffic to the network is end-to-end encrypted. Each machine or phone that pairs is a Device and records its OS (`macos`, `linux`, `windows`, `ios`, `ipados`, `android`). Desktop Devices are Runners: you can create a bot for any paired Runner, and that bot uses the account's encrypted provider credentials there. Phones and tablets are Devices, not Runners.

## Stack

- desktop app for macOS, Windows, and Linux: Go on MyGo's native UI (`desktop/`)
- the AppKit macOS app (`macos/`) is deprecated, replaced by the desktop app; its code stays for now (`bun run mac:dev`)
- CLI: Rust websocket service (`crates/cli`)
  - agent loop after pi-agent-core (`crates/agent`), with Grok-style bot orchestration
  - identity, pairing, E2E (X25519/Ed25519 key pairs + XChaCha20-Poly1305 DEK); signed requests and opaque blobs to the relay
- relay: Rust, axum, SQLite or Postgres (`crates/relay`)
- phone app: Expo (React Native, expo-router) for iOS and Android (`mobile/`); pairs as a Device and speaks the relay protocol itself

## Run it

```bash
bun run dev          # the desktop app (desktop/) with the CLI, rebuilt on change; runs a relay on 0.0.0.0:8787
bun run build        # the desktop app's release builds (bun run build linux/amd64,windows/amd64)
bun run mobile:dev   # the phone app on the iOS Simulator (bun run android for Android)
bun run relay        # a local relay on 0.0.0.0:8787 (set LORCA_RELAY_URL to use it)
cargo test           # agent loop, crypto, and SSE tests
bun run reset        # stop everything and wipe identity, credentials, chats, prefs (-y, --build, --relay)
```

## AI services

- API key: DeepSeek, Anthropic, OpenCode Zen, OpenCode Go (with its Go subscription)
- Subscription: ChatGPT, Grok (SuperGrok or X Premium+)

Architecture: [ARCHITECTURE.md](./ARCHITECTURE.md), the overview, and one doc per subject in [docs/architecture](./docs/architecture). Read the overview and the subjects you touch before implementing.

## Phone app

`mobile/` is an Expo app for iOS and Android. It pairs with an existing Device by scanning a QR code from the desktop app or pasting a string from `lorca pair`, then talks to the relay directly with the Rust Device core: the same keys, envelopes, and jobs as the CLI. A phone is a Device, never a Runner: it reads and writes chats, attaches photos and files, dictates messages, configures the account's provider credentials, creates bots for your Runners, and starts turns; the bots keep running on your machines.

```bash
bun run mobile       # Metro for the dev client
bun run mobile:ios   # build and run on the iOS simulator (Xcode)
```

## Website

The landing page lives in `web/` (TanStack Start on a Cloudflare Worker, shadcn/ui). The product docs at `/docs` are MDX files in `web/content/docs`, rendered with Fumadocs.

```bash
bun run web          # dev server on http://localhost:3000
```

The production Worker deploys from the `website` branch; Cloudflare builds it on every push there. To ship the site, push main to it:

```bash
git push origin main:website
```

## License

[GPL-3.0](./LICENSE)
