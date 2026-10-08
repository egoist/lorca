# Native Go/MyGo workflow feedback evidence

These are the actual Windows/Linux Go view functions rendered by MyGo's offscreen
`ui.NewTester` on the macOS development host. The model uses an in-memory synthetic
CLI transport and the public demo account/messages. No real account, credentials,
private chat, model service or relay is connected. These are native toolkit frames,
not Windows/Linux desktop-session captures or packaged-installer verification.

The screenshot tests exercise the message menu, persistent fields and the actual
`desktop/model` request adapters. They check source ids, edit before/after payloads,
stable retry event ids, displayed diff/current hashes, explicit `null` to disable
review, scoped exclusions and late-reply fencing. Acceptance, rollback and exclusions
receive fixture responses; the tests do not prove Runner persistence or sibling
review/playbook consolidation. Images are unretouched PNGs at 1180×980 DIPs.

| Capture | State |
| --- | --- |
| [Capture](desktop-feedback-capture.png) | User edited, an existing routine target, a corrected draft and a sensitive-material exclusion switch. Switching kind preserves the typed draft. |
| [Proposal](desktop-feedback-proposal.png) | Evidence, originating-work links, a visible diff and guarded acceptance/rejection. Periodic review is explicitly off. |
| [Rollback](desktop-feedback-rollback.png) | After a synthetic acceptance response: version 1 and its reversal preview. The real button submits the displayed current hash. |
| [Exclusions](desktop-feedback-exclusions.png) | Excluded edit text is absent, the chat's exclusion control is disabled, and no proposal waits. The neutral alert belongs to another included demo group. |
| [Conflict](desktop-feedback-conflict.png) | A changed-target error leaves the proposal visible with its original diff guard; nothing is applied optimistically. |

Reproduce from the repository root, after creating the output directory:

```sh
cd desktop
LORCA_RENDER="$PWD/../docs/evidence/issue-86/native-desktop" \
  go test . -run Feedback -count=1 -v
```

The assets remain under `docs/evidence/`, outside production resources/bundles.
Real Windows/Linux interaction and window-system behavior, packaged installs,
live providers, relay/paired-Device decisions and the PR's sibling integration
requirements remain untested. The earlier AppKit images and their captioned limits
remain in the parent evidence directory.
