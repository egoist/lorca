# Native Go/MyGo template sharing evidence

These are the production Windows/Linux Go/MyGo views drawn by `ui.NewTester` at 1180×900 on the macOS development host, in light and dark appearances. They show the initial and reviewed states of the current implementation, not a prior UI version. The sheet receives synthetic CLI replies through the real `desktop/model` request/callback path. The dimmed conversation/sidebar are the repository's mock demo data, and the template uses a fictional Release Reviewer, reserved `review@example.test` address and simulated ready GitHub connection. No real credentials or private chats are used.

| Capture | State |
| --- | --- |
| [Selection, light](desktop-template-export-selection-light.png) | Categories start unchecked; Preview and review acknowledgment are disabled until selection. |
| [Reviewed export, light](desktop-template-export-review-light.png), [dark](desktop-template-export-review-dark.png) | Explicit profile/memory/routine/service selection, personal-content warning and acknowledgment before private saving. |
| [Connection required, light](desktop-template-import-needs-connection-light.png) | No implicit recipient connection; creation stays disabled. |
| [Reviewed import, light](desktop-template-import-reviewed-light.png), [dark](desktop-template-import-reviewed-dark.png) | Edited name persists, recipient connection is selected explicitly, review is acknowledged and paused-routine wording remains visible. |
| [Capability blocker, dark](desktop-template-import-missing-capability-dark.png) | A file with a reusable skill reports missing playbook capability and prevents creation. |

Reproduce from `desktop/` after creating the output folder:

```sh
mkdir -p ../target/desktop-template-images
LORCA_RENDER="$PWD/../target/desktop-template-images" \
  go test . -run 'TemplateDesktop|TemplateFilename' -count=1 -v
go test ./model -run Template -count=1
```

Seven native action/filename tests cover selection, preview/review invalidation, typed name state, explicit mappings, independent DM opening, unsupported versions/capabilities, superseded/dismissed replies, inspector/menu entry and extension-induced replacement confirmation. Four wire/store tests cover full preview content, parameter snapshots, ordered reply reconciliation and errors without new records. Native picker choices and CLI responses are fixtures; these captures do not exercise OS Save/Open dialogs, a live CLI, relay/OAuth or actual Windows/Linux UI rendering. Windows/Linux amd64 executables are cross-compiled separately. Installers/updaters and combined sibling workflows remain untested. Images are visually inspected and kept outside production resources/bundles.
