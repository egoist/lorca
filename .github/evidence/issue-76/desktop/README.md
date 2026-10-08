# Native Go/MyGo desktop attention evidence

These PNGs are actual frames from the production Go/MyGo attention sheet, drawn through `ui.NewTester` on macOS using the native desktop toolkit. They show the shared Windows/Linux view code, not a browser or recreated mockup. They do not demonstrate a live Windows/Linux system window, OS notifications or a remote Runner.

The synthetic projection is the existing `../fixture.json`; the background chats are the application's seeded demo. No credentials, private chats, provider calls or relay connection is used. The native action test clicks the toolbar, changes summaries and selects Scout as an ordinary coordinator, resolves an item, and follows source links. Additional wire/action tests check CLI method/revision/null payloads, ordered main-thread callbacks and stale-refresh races. Notification tests check summary/urgent/quiet and watched/read distinctions without posting real OS alerts.

- `desktop-attention-active-light.png` / `desktop-attention-active-dark.png`: current brief, urgent blocker and pending review with next actions and source references.
- `desktop-attention-followups-light.png` / `desktop-attention-followups-dark.png`: synthetic remaining commitments/changes after removing the review/blocker, summaries off and Scout selected as the default coordinator. Existing items and the retained brief still belong to their recorded coordinator, Chef.
- `desktop-attention-resolved-light.png` / `desktop-attention-resolved-dark.png`: empty active projection with the current brief retained and summaries off.

From `desktop/`:

```sh
LORCA_RENDER=../.github/evidence/issue-76/desktop go test ./... -run TestAttention -count=1
```

`go test ./...` and `go tool mygo vet .` check the full native app. Windows amd64 and Linux amd64/arm64 compilation is separate evidence; live platform notification/click/paired-Device/provider flows and sibling consolidation remain PR draft requirements. These evidence assets stay outside production resource bundles.
