# PR #101 UI evidence

These PNGs show the production AppKit `BudgetViewController` and
`ConnectorLimitsViewController` from the #79 branch. The opt-in XCTest harness
creates native fixture windows in Aqua appearance, waits for the controllers to
read synthetic localhost websocket responses, and captures each content view
with `NSView.cacheDisplay` / `NSBitmapImageRep` at the window's 2× backing scale.
The fixture windows are not ordered onto the user's desktop.

| Capture | State shown |
| --- | --- |
| [Task allowance](task-allowance.png) | Spending, tokens, runtime, retry and connector-call limits; separate API spending, subscription API-equivalent estimates and unknown-price call counts |
| [Routine exhausted](routine-budget-exhausted.png) | Orange exhaustion state, exhaustion reason, consumed allowance and explicit increase/resume or renew actions |
| [Task interrupted](task-interrupted.png) | Restart/interruption state and the instruction to check completed effects before explicit recovery |
| [Account cooldown](connector-account-cooldown.png) | Account call rate/window/concurrency controls and a synthetic service cooldown |
| [Shared service](connector-service.png) | The same controller switched to its service scope, with shared service limits and active-call count |

All data is synthetic: “Demo Assistant”, “Screenshot Runner”, public fixture ids,
usage values and cooldown timestamps. The server accepts only fixture info and
read-only budget/connector queries. The harness connects the app's CLI client to
that endpoint without starting the app store or the real CLI. Captures contain
no credentials, private chats, provider requests or relay/service connections.

The Task allowance image uses the implemented `taskID:` constructor hook. Its
Task-card navigation still depends on #72 consolidation, as the PR states.
These captures demonstrate native rendering and the connector scope selection;
they do not exercise Runner budget mutations, model resumption, external OAuth,
paired-device transport, or the unresolved combined task/routine/event paths.

Regenerate on macOS with Bun, Xcode and Swift installed:

```sh
bash evidence/issue-79/capture.sh
```

The script starts a temporary read-only fixture server on an ephemeral localhost
port, builds the Markdown FFI if missing, runs the single opt-in capture test,
and stops the fixture server. The harness skips during ordinary test runs unless
`LORCA_CAPTURE_DIR` is set. Assets and capture scripts stay in this evidence
directory; test code stays under `macos/Tests`, outside production bundles.

## Native Windows/Linux UI

The `desktop-*.png` captures show the production Go/MyGo views in
`desktop/sheet_budget.go`, `sheet_connector_limits.go`, and `inspector.go`.
MyGo's `ui.NewTester` draws complete native view frames offscreen on the macOS
development host at 1180×850. These are captures of the shared native toolkit
used by the Windows/Linux app, rather than Windows or Linux desktop sessions.
Their captions do not assert platform runtime or installer QA.

| Capture | State shown |
| --- | --- |
| [Inspector](desktop-inspector-pricing-recovery.png) | DM Budget → Manage entry point, separate spending/unknown labels, and routine exhaustion ahead of enabled state |
| [Routine exhausted](desktop-routine-budget-exhausted.png) | All five limits, usage, an exhaustion reason, and explicit increase/resume or renew actions |
| [Task interrupted](desktop-task-budget-interrupted.png) | The canonical task constructor hook, interrupted state and completed-effect guidance |
| [Account cooldown](desktop-connector-account-cooldown.png) | Account call rate/window/concurrency and the service cooldown |
| [Service scope](desktop-connector-service.png) | Service scope selected through the native menu, shared limits and active-call count |

The Go tests replace transport with a synthetic in-memory fixture, seeded public
demo conversations, example counters, and no real credentials or network
connections. Its requests and responses use the implemented CLI methods; action
tests check submitted limits, Save-before-Resume ordering, `run: false` for tasks,
renew confirmation, failure retention, persistent scope drafts, and stale replies.
The canonical Task-card opener remains a #72 consolidation hook. These tests
do not exercise an actual Runner, provider, relay, OAuth service, or owning
task/event execution.

Regenerate from the worktree root with Go installed:

```sh
cd desktop
LORCA_RENDER="$PWD/../evidence/issue-79" go test -run 'TestDesktop(Budget|Connector)' -count=1 .
```

The test harness lives in `desktop/sheet_budget_test.go`; assets remain in this
evidence directory, outside production bundles.
