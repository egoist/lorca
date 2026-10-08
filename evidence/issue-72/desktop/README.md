# PR #102 native Windows/Linux task UI evidence

These PNGs capture the implemented Go/MyGo task views with `ui.NewTester` on the macOS development host. They are offscreen native view frames, without Windows/Linux window-manager chrome. The list uses a 360 × 600 frame, the complete editor uses 1180 × 1450, and the stale-edit/Reload sequence uses 1180 × 850. The taller frame makes the complete scrolling form visible in one image; the ordinary-sized frame shows that the status and actions stay visible when the body scrolls.

`desktop/sheet_task_test.go` drives the production inspector and task sheet over the repository's synthetic demo store. It uses fixture task ids, bots, Runners, chats and `example.invalid` links. The Save/Reload sequence goes through the real task request handlers with in-memory mock responses, including a revision conflict. No real credentials, private chats, live CLI, providers, relay, or external services participate. These captures are evidence of native rendering and fixture-driven actions, not live Windows/Linux or paired-Device validation.

| Image | Captured state |
| --- | --- |
| [Task list, light](desktop-task-list-light.png) / [dark](desktop-task-list-dark.png) | The actual Tasks inspector card shows all six states and owners, with Open and Create controls. |
| [Blocked work](desktop-task-blocked-light.png) | The native task editor shows owner/Runner, linked chats, the saved primary run chat, acceptance criteria, next action, and a seeded restart blocker. |
| [Result and evidence](desktop-task-awaiting_review-dark.png) | The native editor shows an awaiting-review result and HTTPS proof. Start saved task is disabled while review is pending. |
| [Stale edit](desktop-task-stale-light.png) | Save uses revision 7 after fixture revision 8 changes ownership. The editor retains the unsaved goal and old owner; its conflict status and Reload remain visible. |
| [After Reload](desktop-task-reloaded-light.png) | The same native editor deliberately replaces its draft with revision 8, including the updated goal, owner and Runner. This is the after-state of Reload, not a pre-implementation comparison. |

The optional budget opener remains unregistered until #79/#101 consolidation. The tests verify its owner/Runner/task/chat arguments and refuse mismatches, but these images do not exercise the budget sheet. Output/review/handoff/attention/budget/project/event consolidation, actual CLI task mutations, live paired transport/provider effects, Windows/Linux runtime/installation and signed release flows remain untested together. Build checks use MyGo's cross-platform frontend build with `-skip-build-command`; Windows NSIS installer and update signing are unavailable, and release CLI resources are not bundled by that check.

To reproduce the captured states from this checkout:

```sh
cd desktop
LORCA_RENDER="$PWD/../evidence/issue-72/desktop" \
  go test . -run 'TestTaskEditorStaleRevisionKeepsFormAndReloadsAuthority|TestRenderDurableTaskDesktopStates' -count=1
```

The harness writes both appearances for each captured state. This directory retains the six inspected images linked above; alternate captures are not shipped. Evidence assets remain outside `desktop/assets` and all production bundles. The earlier AppKit screenshots and their capture method remain in [the parent README](../README.md).
