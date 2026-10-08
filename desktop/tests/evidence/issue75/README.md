# Native desktop Access evidence

These frames render the Go/MyGo views used by Lorca on Windows and Linux. They come from the native `ui.NewTester` offscreen renderer on macOS, with synthetic demo bot/chat data and invented Gmail Work/Personal instance IDs. They are not screenshots from live Windows/Linux installations and contain no real credentials or private chats.

- `desktop-access-profile-before-*.png`: the native Profile card with the default full-access summary.
- `desktop-access-profile-after-*.png`: the same card after an explicit fixture deny-all connection/tool policy with shell disabled.
- `desktop-access-connections-*.png`: the Access sheet with Personal capabilities disabled; Work Read/Draft enabled and Write/send_message excluded; local tools use an explicit selection.
- `desktop-access-local-controls-*.png`: the sheet scrolled to its local tool choices, filesystem Read, shell disabled, and the isolation explanation.
- `desktop-access-refusal-*.png`: the native transcript's refused-access card with Edit Access and Dismiss, without Allow once/Always allow.

Each view has light and dark captures. These are direct native tester frames without raster edits or generated artwork. The action tests click instance/capability/tool checkboxes, select filesystem access from the menu, change shell access, save the profile, and open/dismiss refusal requests. Other tests cover catalog/language rebuild state, dismissed-sheet reply lifetimes, explicit empty allowlist wire encoding, ordered CLI replies and newer roster policy winning over stale save responses.

From `desktop/`:

```sh
mkdir -p tests/evidence/issue75
LORCA_RENDER=tests/evidence/issue75 go test . -run 'TestNativeAccessChoicesPersistAndSave|TestAccessCardOpensEditorWithoutGrantingTheCall|TestRenderDesktopAccessProfile' -count=1
```

Desktop Go tests and MyGo vet pass. Windows amd64 and Linux amd64/arm64 Go binaries cross-compile with CGO disabled. Actual Windows/Linux interactive runs, installer packaging, production-account OAuth, live/paired CLI Access workflows and combined sibling execution remain untested. The CLI remains the authority for policy execution; this UI adds no sandbox, provider credentials, accounting store or permission override.
