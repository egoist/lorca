# PR #102 native AppKit evidence

The native Go/MyGo parity follow-up has separate [Windows/Linux task UI captures and reproduction notes](desktop/README.md).

These PNGs capture the existing AppKit controllers from the #72 implementation (`724b769`) with synthetic fixtures. They are offscreen `NSView` bitmap captures at 2× scale in Aqua appearance, with transparent view gaps composited over AppKit's standard window background. They show application content rather than desktop/window-manager chrome. The chat comes from the repository's `MockData`; tasks and the revision conflict come from the test fixture. No real credentials, private chats, provider calls, or relay traffic participate.

| Image | Captured state |
| --- | --- |
| [Group tasks](group-task-states.png) | Actual `ChatViewController` and `InspectorViewController`, with all six task states beside synthetic group work; Open and Create remain the implemented controls. |
| [Restart blocker](blocked-after-restart.png) | Actual `DurableTaskViewController`, scrolled to next action and required reason for an interrupted run. |
| [Review evidence](review-result-evidence.png) | The same native editor, scrolled to result and supporting evidence; the HTTPS URL uses `example.invalid` and Start saved task is disabled while awaiting review. |
| [Stale edit](stale-edit-keeps-form.png) | The real editor Save handler receives a scripted CLI error (`expected 7, current 8`); the form, linked chats, criteria, and Reload action remain visible. |

The fixture captures document UI rendering and the local error path. The existing PR's sibling-consolidation and live paired-Device/provider/end-to-end limitations still apply. These are current-implementation state captures, not a pre-implementation before/after comparison.

To reproduce from the assigned checkout with the normal Swift/Markdown build prerequisites and Python's `websockets` package available:

```sh
python3 macos/Tests/Fixtures/DurableTasks/capture.py --output evidence/issue-72
```

The runner binds an ephemeral loopback websocket, sets `LORCA_MOCK=1`, and runs the opt-in `DurableTaskCaptureTests` harness. It never launches a Lorca CLI or opens a user window. The test is skipped during ordinary test runs. Fixture code lives under `macos/Tests/`; evidence lives here, outside production bundles.
