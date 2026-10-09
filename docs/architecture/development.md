# Repo layout and development

Where each crate and app lives in the repository, and the development loops and builds that run them. The Windows and Linux app's own loop and builds are in [Windows and Linux app](desktop-app.md).

## Layout

```
lorca/
  ARCHITECTURE.md      # the overview and the list of subjects
  README.md
  docs/architecture/   # one doc per subject
  docs/agent/          # lorca-agent's own documentation
  Cargo.toml           # workspace
  crates/agent/        # lorca-agent: loop, tools, codemode (QuickJS), and Messages, Chat Completions, Responses, ChatGPT, and Grok providers
  crates/models/       # lorca-models: the model catalog (catalog.json: windows, thinking levels, rates), on every Device and in every app's pickers; lorca.app serves it so Devices update without a release
  crates/provider-auth/ # OAuth token types and PKCE flows shared by every Device
  crates/tls/          # lorca-tls: the certificate trust of every Device's HTTPS, the system's on macOS and Windows
  crates/cli/          # lorca: the Device core as a library (keys, relay sync, jobs, the JSON API) + runner and server features + the binary
  crates/mobile/       # lorca-mobile: the core for the phone over UniFFI
  crates/markdown/     # lorca-markdown: message Markdown as the blocks and spans every app renders (pulldown-cmark, and GitHub's autolinks for bare URLs and addresses), for the Mac and phone over UniFFI
  crates/relay/        # lorca-relay: axum + SQLite or Postgres, and its Dockerfile
  macos/               # AppKit SPM app; the build bundles the CLI
  desktop/             # the Windows and Linux app: Go on MyGo's native UI; the build bundles the CLI
  mobile/              # Expo app for iOS and Android: a paired Device over the core (modules/lorca-core)
  web/                 # the site
  scripts/             # bun scripts: dev loop, bundle build, macOS and phone releases, the desktop app's dev loop and builds, string and doc checks
  .github/workflows/   # release-cli.yml, release-mac.yml, release-desktop.yml, and release-mobile.yml: release builds; test.yml: every app's and crate's tests on each pull request; docs.yml: the doc check
```

## Development loops and builds

`bun run android` rebuilds the Rust core for Android, then builds and runs the Expo dev client on the Android emulator. `cd mobile && bun run core` rebuilds the Rust core for both phone platforms; `bun run mobile:dev` is the iOS development loop described below.

`bun run dev` rebuilds the CLI and Lorca Dev on Rust or Swift changes (the generated markdown bindings under `macos/Sources/LorcaMarkdown` are left out of the watch, and rewritten only when they differ) and relaunches the app through `open`, so the app is its own responsible process for TCC: a binary spawned from the terminal is charged to the terminal app, whose Info.plist decides whether a microphone or speech request aborts. `bun run build` produces the Lorca release bundle. The bundle step restamps the app binary's SDK version (`stampSDK` in `scripts/app.ts`, through `vtool`): the Swift Build engine writes the deployment target (14.0) there, and AppKit gives a binary stamped below the macOS 26 SDK its older look, with a flat sidebar and an opaque titlebar strip. `bun run relay` runs a local relay. `bun run mobile:dev` (`scripts/mobile.ts`) is the Lorca Dev phone loop on the iOS Simulator, or on `--device <name or udid>`: it fingerprints the crates the phone links, the prebuild inputs (Expo config, assets, `package.json`, plugins, targets), the pod inputs with the checkout's path, and the native module sources (stamps in `mobile/.expo/dev-stamps.json`), rebuilds what is stale (`bun run core ios`, a clean `expo prebuild`, `pod install`, `expo run:ios`), starts Metro, and opens the dev client on it. A Rust save while it runs rebuilds the core and installs the app again. The Mac and phone loops leave production Lorca processes alone, so all builds run side by side.
