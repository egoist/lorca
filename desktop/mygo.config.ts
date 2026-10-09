// Lorca for Windows and Linux, an app of native UI. `go tool mygo dev` builds Lorca Dev
// (app.lorca.dev), which keeps its account in ~/.lorca-dev and its CLI on port 4863, apart from an
// installed Lorca.
export default ({ command }: { command: string }) => ({
  name: "Lorca",
  identifier: "app.lorca",
  // The app's own version, apart from the Mac app's (the root package.json's). `bun run
  // release-desktop` and the Release desktop workflow read it here.
  version: "0.1.6",
  icon: command === "dev" ? "assets/icon-dev.png" : "assets/icon.png",
  out: "build",
  // Open in Lorca on a shared bot's page (lorca.app/t/…). The installer registers the scheme on
  // Windows, the desktop entry on Linux, and Info.plist on macOS; Lorca Dev answers its own.
  urlSchemes: [command === "dev" ? "lorca-dev" : "lorca"],
  // Release builds update themselves from this repository's newest release tagged
  // desktop-v<version>, which MyGo finds through the GitHub API since the latest release is the
  // CLI's, and install only what the key of `mygo keygen` signed. `bun run release-desktop`
  // uploads a release as a draft: docs/releasing-desktop.md. The version's section of CHANGELOG.md
  // here is the update's release notes.
  updates: {
    publicKey: "WzJsOGNIuf6mcEqo5ff8jub+NoQQOEk4JXreLPYjgyQ=",
    github: "egoist/lorca",
    tagPrefix: "desktop-v",
  },
  linux: {
    comment: "Chat with your bots, which run on computers you own",
    categories: ["Network", "Chat"],
  },
});
