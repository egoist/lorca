# Native Go/MyGo playbook evidence

These frames render the actual desktop main window and playbook sheets through MyGo's native `ui.NewTester`, using the app's synthetic demo plus a fixture transport for the existing `playbooks.*` CLI methods. The images cover the Windows/Linux Go/MyGo implementation and are rendered on a macOS host; they do not claim live Windows/Linux window, file-dialog, accessibility, or compositor execution.

The native interaction tests select tabs, edit persistent fields, rename/add/remove bundled files, inspect read-only history, choose scope and sources, handle guarded-write conflicts, and confirm that capture only drafts until reviewed Save. The fixture uses public-demo messages and synthetic ids/hashes. It makes no real provider request, reads no account credentials or private chats, and never executes the displayed script.

Light/dark frames include:

- `desktop-playbook-manager`: scoped metadata listing, draft marker, and the inspector's Manage entry.
- `desktop-playbook-draft`: bot-scoped draft review, instruction editor and manual Save Skill.
- `desktop-playbook-references`: bundled text and relative filename after edits.
- `desktop-playbook-scripts`: optional script text and existing-permissions note.
- `desktop-playbook-history`: retained revisions, source notes/ids, and read-only text selection.
- `desktop-playbook-workflow-capture`: the selected request and completed bot reply before drafting.
- `desktop-playbook-correction-validation`: one source correction is refused locally; two selected sources can draft.
- `desktop-playbook-conflict`: guarded save rejection preserves the draft and offers Keep Draft or Reload.

Reproduce from `desktop/` with a pre-existing render directory:

```sh
LORCA_RENDER="$PWD/../docs/evidence/issue-77/desktop" \
go test . -run PlaybookNative -count=1
```

The feature tests and fixture transport are in `desktop/sheet_playbooks_test.go`; model contract/post-queue tests are in `desktop/model/playbooks_test.go`. The PNGs stay under `docs/evidence`, outside production bundles. The original AppKit evidence remains in the parent folder.

Direct Go builds pass for Windows amd64 and Linux amd64/arm64. Live platform GUI/file-panel behavior, actual paired-Device relay sync, live drafting providers and combined sibling seed/import/feedback flows remain review limits in PR #93.
