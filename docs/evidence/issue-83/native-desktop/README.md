# Native Go/MyGo project context evidence

These captures render the Windows/Linux app's production Go/MyGo views in `ui.NewTester`, on a macOS test host. The native tests drive entry selection, source refresh, and revision-history controls over synthetic demo chats and a test-only local-CLI transport. They contain no real account data, credentials, private conversations, provider connection, or relay traffic.

| Capture | State |
| --- | --- |
| [Inspector before](desktop-project-navigation-before.png) | The native Group section at the merged main foundation, before the project context row. |
| [Inspector after](desktop-project-navigation-after.png) | The added Project context row opens the editor from a group. |
| [Agreed decision](desktop-project-decision.png) | Current decision, provenance, verification time, immutable output-version reference, and correction controls. |
| [Unavailable source](desktop-project-unavailable.png) | Refresh records a simulated source failure, retains its prior snapshot, and exposes the retrieval time and error. |
| [Reference asset](desktop-project-asset.png) | Project file metadata and the Open asset action. A separate action test checks its unavailable/retry error, without opening a real file. |
| [Revision history](desktop-project-history.png) | A superseded decision is selected; editing, Save, and Remove are disabled. |

The capture tests are in [sheet_project_context_test.go](../../../../desktop/sheet_project_context_test.go). Generate the current frames from the repository root:

```sh
mkdir -p docs/evidence/issue-83/native-desktop
LORCA_RENDER="$PWD/docs/evidence/issue-83/native-desktop" \
  go -C desktop test . -run '^TestRenderProjectContext' -count=1
```

The before frame uses Go's source overlay with `desktop/inspector.go` from feature merge commit `85e3c14` (the specified main foundation is its second parent). The other code and synthetic demo data stay the same. An overlay changes the test build's source selection and does not change the checked-out main or feature files. Set `LORCA_PROJECT_NAV_BEFORE=1` and run only `TestRenderProjectContextNavigation` with that overlay to regenerate the before image.

These are native toolkit fixtures, not screenshots taken on a running Windows or Linux desktop. Cross-builds cover Windows amd64 and Linux amd64/arm64. Real platform launch, system file dialogs/file opening, live paired-Device/provider traffic, and combined sibling integration remain outside these captures and retain the PR's stated limitations. All PNGs are outside production bundle resources.
