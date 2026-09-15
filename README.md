# Tinybot

Tinybot is a Grok bot alternative: a native macOS AppKit app and a Rust CLI. The CLI is a localhost websocket service and the agent loop; the app is the UI for that CLI.

You create bots, talk to them 1:1, or put them in a group chat. Bots can hand work to each other and orchestrate, in the same spirit as Grok Bot.

The UI is AppKit (SPM), built to feel like a Mac app: materials, density, keyboard, and motion.

Identity is a local key pair. Computers pair to each other. Traffic to the network is end-to-end encrypted; the website is a [Happy](https://happy.engineering/docs/security/)-style relay for ciphertext. Each machine running the app is a Computer. You can create a bot for any paired Computer; that bot’s DeepSeek key or ChatGPT subscription lives on the assigned Computer.

## Stack

- macOS app: AppKit, SPM
- CLI: Rust websocket service
  - agent loop after vercel-labs/fx, opencode, pi-agent, and Grok-style bot orchestration
  - identity, pairing, E2E (NaCl-style key pairs + DEKs); signed requests and opaque blobs to the relay
- website / relay: TanStack Start/Query, TailwindCSS, shadcn/ui, Cloudflare Worker, Drizzle ORM, Drizzle Kit

## AI services

- API key: DeepSeek
- Subscription: ChatGPT

Architecture: [ARCHITECTURE.md](./ARCHITECTURE.md). Read it before implementing.

## Later

- A mobile app
