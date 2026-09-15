# Tinybot

Tinybot is a Grok bot alternative, implemented as a fully native macOS app using AppKit, and a CLI written in pure Rust providing a websocket service, the app is just a UI for the CLI.

Basically it's a beautiful and native-look chat ui like Grok bot, you can create bots and talk to them directly or create a group chat to talk with many bots, bots can also delegate works to other bots, and bot orchestration and what not, basically match Grok bot features.

It's focused on performance, written fully in AppKit, NO SwiftUI. And the app strikes to feel native, with careful and elegant UX design, extrodinary user experience.

It needs an account system, and the app needs logged in. Every computer the app is running on will be registered as a Computer, and user can create a bot running on any registered computer.

## Stack

- macOS app: AppKit, no Swift UI, SPM
- websocket service: Rust CLI
- website: Tanstack Start/Query, TailwindCSS, Shadcn UI, Cloudflare Worker

## Future plan

- A mobile app
