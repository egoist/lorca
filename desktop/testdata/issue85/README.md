# Native Go browser session evidence

These frames come from `ui.NewTester` rendering the real native Go/MyGo sheet and main window with the in-process synthetic demo on the macOS development host. They show the shared desktop UI used by Windows/Linux builds; they are not live Windows/Linux screenshots. The fixtures contain no real credentials or private chats and open no browser. Images live in testdata and are not embedded in production bundles.

- `browser-bot-control-light.png`: bot control before takeover.
- `browser-human-control-light.png`: human control with explicit Return to Bot.
- `browser-takeover-waiting-light.png`: Stop remains available while takeover waits; competing input controls are disabled.
- `browser-paired-device-light.png`: a paired capability response disables visible Open and offers Pause on Runner, with local interaction limits.
- `browser-human-control-dark.png`: the same native controls in dark appearance.

From desktop/, create the output folder and run:

```sh
mkdir -p .mygo/issue85-evidence
LORCA_RENDER="$PWD/.mygo/issue85-evidence" go test ./... -run Browser -count=1
```

The focused suite drives native field edits and Create/Open/Take Over/Return/Stop actions, verifies CLI wire arguments and explicit capability limits, and tests late polls/takeover replies after Stop and replies after dismissal. Screenshot transport is simulated in wire tests. These fixtures do not establish live Windows/Linux browser control, paired-device relay operation, or #75/#80 combined-workflow coverage.
