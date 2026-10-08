# Native desktop handoff evidence

These are unedited PNG frames from the Windows/Linux application's production Go/MyGo views, rendered by `ui.NewTester` on the macOS development host. They are native toolkit captures, not web pages or drawings; native Windows/Linux window-manager chrome and actual Windows/Linux runtime execution are outside the offscreen fixture.

`desktop/message_links_test.go` supplies synthetic Chef/Scout bots and completion/blocker messages. A fake local CLI transport returns real `chats.messages` JSON shapes through the store's asynchronous ordered main-thread post queue. The tests click the rendered link glyphs, select Scout, apply two older pages using their `before` message ids, and reveal the referenced response. No account, provider, relay, real credential or private chat is used. Sample report/evidence claims are seeded text; the capture does not run parser checks or a specialist agent.

- `desktop-handoff-completed.png`: requester chat before clicking the result link.
- `desktop-handoff-loading-history.png`: destination chat while the missing result is fetched from older transcript pages.
- `desktop-handoff-opened-response.png`: referenced response revealed after both pages arrive; the scrollbar and newer context rows show that navigation does not stay at the newest message.
- `desktop-handoff-unavailable.png`: native alert after the history is exhausted without the referenced message.
- `desktop-handoff-blocked.png`: independent seeded blocker report with an internal request link.

From the assigned worktree:

```sh
mkdir -p docs/pr-evidence/issue-74/desktop
cd desktop
LORCA_RENDER="$PWD/../docs/pr-evidence/issue-74/desktop" \
  go test ./... -run 'TestHandoff|TestHistory|TestParse' -count=1
```

Additional tests cover request sharing, duplicate-row handling, RPC failure and Retry, changed transcript/account rejection, navigation away and replacement links, target messages arriving through sync, hidden tool references, opaque percent-encoded ids, Chinese loading text, and the unchanged http(s)/mailto system-opener allowlist. The assets are outside production bundles. Live local-CLI/relay history backfill, model/provider outcomes, physical Windows/Linux execution, packaging/installers and sibling task/output/attention consolidation remain separate validation requirements.
