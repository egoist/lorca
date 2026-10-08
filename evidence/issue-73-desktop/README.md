# Native desktop review queue evidence

These images use the production Go/MyGo review inspector and sheet through `ui.NewTester`, rendered offscreen on the macOS development host. The same Go UI cross-builds for Windows amd64 and Linux amd64/arm64. All data, including background conversations and credential status labels, comes from the repository's fictional demo and synthetic sandbox review items; no real account or private chat is loaded.

The focused native action tests actually type/edit, save, approve, reject, cancel and reload the mock projection. They verify the unsaved-edit guard, exact 64-bit JSON ids/key spelling, synced-version conflicts, disabled uncertain-state mutations and dismissal while a reply is queued. The wire tests separately verify real CLI method/parameter shapes and ordered replies. The pictures are not physical Windows/Linux captures, real paired-Device/OAuth actions or a live restart test.

| Image pair | State |
| --- | --- |
| `review-inspector-light.png`, `review-inspector-dark.png` | Native inspector entry with a pending review and Review… action. |
| `review-draft-light.png`, `review-draft-dark.png` | Saved version 1 draft before local edits. |
| `review-unsaved-guard-light.png`, `review-unsaved-guard-dark.png` | After editing and attempting Approve: Save Changes is required first. |
| `review-proposed-call-light.png`, `review-proposed-call-dark.png` | Edited/saved version 2 call; integer tokens and nested keys remain exact. |
| `review-sync-conflict-light.png`, `review-sync-conflict-dark.png` | A simulated paired projection changes: local text/version remain until Reload. |
| `review-uncertain-light.png`, `review-uncertain-dark.png` | Fixture-supplied uncertain outcome with disabled mutations and available Reload. |

Regenerate from the worktree:

```sh
mkdir -p evidence/issue-73-desktop
cd desktop
LORCA_RENDER="$PWD/../evidence/issue-73-desktop" \
  go test . -run 'TestReview|TestInspectorReview' -count=1
```

Capture assets are outside production resources/bundles; Go tests are excluded from production builds. Windows NSIS installation, signing/updater delivery, physical platform interaction and live/consolidated Runner permission flows retain the PR's documented limits.
