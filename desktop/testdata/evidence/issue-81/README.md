# Native Go/MyGo evidence for #81 / PR #94

These PNGs are frames from the implemented Windows/Linux native view functions,
rendered by MyGo's `ui.NewTester` on the macOS build host. They use the native
Go/MyGo toolkit, not a browser or an HTML mockup. The tests drive the real view
functions with native click/type/menu actions over the model's synthetic demo.
They do not execute the Windows/Linux window manager, installers, live CLI,
OAuth, provider inference, paired-Runner relay or OS service lifecycle.

The underlying chats, accounts, replies and relative times are synthetic demo
content. The inbox recovery frame injects an expired sign-in to show the
implemented recovery controls. The Chinese frame translates client controls;
CLI/catalog/demo content keeps its supplied wording. No real credentials or
private chats are used. Images are stored under testdata, outside production
assets and bundles.

| Image | State |
| --- | --- |
| onboarding-workflow-entry-light.png | The final native onboarding page offers Choose a Workflow beside Open Lorca. |
| workflow-outcomes-light.png | Three packs and an explicit Runner picker. |
| workflow-questions-light.png | Repository scope typed into a persistent field and an existing specialist explicitly chosen. |
| workflow-connection-recovery-dark.png | A selected Personal demo inbox needs sign-in; sample execution is disabled while setup remains recoverable. |
| workflow-before-review-light.png | A completed demo result requires review; schedule activation is absent. |
| workflow-after-review-light.png | After review, the schedule is still paused; enable and finish-paused actions are separate. |
| workflow-cancelled-dark.png | Cancellation retains setup/resources and offers Resume Setup. |
| workflow-chinese.png | The same native setup with Chinese client controls. |

The review pair compares before/after the review state, not releases. The action
tests also check cancellation/reopening, named-instance selection, specialist
selection, dirty-field retention, and deliberate activation. Wire tests check
parameter snapshots and delivery through the main-thread post queue.

Regenerate from `desktop/`:

```sh
mkdir -p testdata/evidence/issue-81
LORCA_RENDER="$PWD/testdata/evidence/issue-81" go test -run '^TestWorkflow' .
```

The renderer emits extra light/dark variants; this commit keeps the eight
listed views, which are visually inspected. Main-window captures are 1180×900;
onboarding is 660×560. Cross-compilation checks cover Windows amd64, Linux amd64
and Linux arm64 UI executables. Physical platform navigation/rendering,
packaging/installation and live/consolidated sibling flows remain pending, as
listed in the draft PR.
