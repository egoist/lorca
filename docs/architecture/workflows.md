# Workflow onboarding

Guided workflows are optional marketplace packs. The bundled index offers meeting preparation, inbox triage and repository monitoring. Each pack names the outcome, its required questions, specialist bot templates, integration service requirements, routine tasks and schedules, and the specialist and task for a sample. The CLI (`crates/cli/src/workflows.rs`) owns progress; the AppKit and native Go/MyGo desktop apps call their local websocket API.

## Packs and setup

The v1 [marketplace index](marketplace.md#the-index) adds a `packs` array beside `plugins` and `bots`. Pack entries have their own `version: 1`; unsupported versions, invalid or repeated IDs, missing specialist templates, and unreadable schedules are skipped per entry. Service requirements use portable `service_id` values. A service manifest can arrive in a later index: setup shows the missing integration and preserves its progress. Older clients read the existing plugin and bot arrays and ignore packs.

`workflows.start { pack_id, runner_id }` creates or resumes one setup per pack and Runner. Its ID is a hash of those two IDs. Starting it pins the pack and specialist profiles, so a marketplace update leaves a partially answered setup's questions and planned schedules stable. A cancelled setup resumes with its saved answers, resource IDs and account selections.

`workflows.configure { id, answers, bot_ids? }` accepts only the pack's required questions, each a nonempty text answer of at most 2,000 characters. Credential-shaped answers are refused; integration sign-in and setup fields hold credentials. Explicit bot choices must belong to the selected Runner. Without a choice, setup keeps its recorded specialist, reuses an existing bot whose description matches the template, or adds the specialist with the account's first connected provider. The profile and playbooks of a reused bot stay intact.

The CLI reuses routines with the same bot, name, canonical schedule and task. Missing routines are created paused. Created bot and routine IDs are deterministic, including the specialist or routine role and assigned bot, so retrying after an interrupted insert finds the same resource. A local async lock serializes setup mutations and installs. The setup records which routines it created; cancellation pauses these and retains the resources for a later resume. A reused user's routine keeps its controls and state.

Answers and selected integration IDs are decoded only for local API replies and model context. A routine or sample turn receives the context of its workflow, including user-supplied scope and the exact selected connection namespaces. Ordinary turns of its specialist can also read the workflow context. Context is scoped setup data; the bot's description, memory and editable playbooks keep their own mechanisms.

## Connections and recovery

Each requirement names a marketplace service. Setup lists matching installed plugins on its Runner, using `PluginStatus.service_id` for named accounts and the plugin ID for singleton plugins. The user selects an instance explicitly; a named account has no inferred default. Servers from `mcp.json` stay in their own management flow.

`workflows.connection { id, service_id, plugin_id? , account_name? }` binds the chosen existing instance. With no instance and no advertised matching account, it calls the existing Runner's `plugins.install` implementation and stores the returned stable instance ID. The pack name is the default account label, so an installation response lost before progress is saved does not ask the Runner for a differently named account on retry. An existing selection is retained even while the Runner's machine advertisement is pending. The install response supplies its immediate status to the page; the encrypted machine advertisement supplies subsequent status. A matching existing account must be chosen before another install.

Sign-in and variable setup use the existing plugin sheet and `plugins.connect` / `plugins.detail` / `plugins.set_variables` paths, with the instance ID. Tokens and integration configuration stay on the assigned Runner; requests to another Runner are sealed through the [plugin protocol](plugins.md#plugins). The setup stores references to Installed records, while the account's model-provider credentials remain in the encrypted `credentials` blob.

An offline Runner, unavailable service, failed sign-in or missing provider leaves setup recoverable. `workflows.get { id }` returns connection choices and progress, specialists, routines, sample replies, whether a sample is running, and the reason a sample or activation is blocked. `workflows.clear_connection { id, service_id }` removes a stale selection without uninstalling the integration. Replacing answers, specialist choices or accounts discards sample review; imported schedules are paused before another sample runs.

## Sample, review and activation

`workflows.sample { id }` requires all selected instances to be ready, valid specialists/routines and a connected provider. It starts a real `workflow_sample` Job in the sample specialist's DM on its assigned Runner. It writes the user request into that chat, uses the ordinary provider, streaming, permission and encrypted relay paths, and keeps the sample's job ID. A repeated request during a live sample returns that generation's progress. A local interrupted sample can be retried; a remote sample waits through its five-minute job-result window before retry is offered.

The Runner records the result boundary after acquiring the chat lock. Completed text replies from that sample become its review result. A failed turn or one with no completed reply is retryable and cannot be reviewed. A sample's direct tools omit bot, routine and integration management; imported routines also enforce their review gate through `routines.set_enabled`. The model receives a request to present a draft in the chat and leave scheduling to the setup page.

`workflows.review { id, job_id }` acknowledges the current completed sample; it requires the result messages to have reached the reviewing Device. Only then do the desktop apps show the schedule choice. `workflows.enable { id }` checks the ready connections and unchanged routine tasks again and enables the pack's schedules. The user can instead finish with imported schedules paused. Starting another sample pauses the routines setup owns and requires another review.

`workflows.cancel { id }` cancels only its sample job, forwards that cancellation sealed to a remote Runner, pauses owned routines and records cancelled progress. The Runner checks the current sample generation and cancellation state again while persisting its outcome, so a late completion cannot revive cancelled or superseded setup.

## Persistence and sync

SQLite's `workflow_setups` table holds only setup IDs, update times and account-encrypted ciphertext. XChaCha20-Poly1305 uses the account DEK and associated data `workflow:<setup id>`; answers, pack/profile snapshots, resource bindings, sample message IDs and review state are inside the ciphertext. The optional `RosterBlob.workflows` extension carries these envelopes inside the encrypted roster, with no new relay blob kind. Records merge independently by setup ID and update time. Records omitted by a build that does not know workflows are retained and republished. Cancellation remains a record, so sync preserves its state. Forgetting the account clears the setup table with the rest of local state.

## AppKit flow

The final onboarding page offers Choose a Workflow… beside Open Lorca. Its marketplace sheet starts with outcome choices. The same Guided Workflows section leads the normal marketplace and appears in search. A workflow page names its fixed Runner, asks only its setup questions, offers existing specialists and explicit integration account choices, opens the existing plugin sheet for sign-in and setup, and shows connection progress and recovery errors. It renders the sample replies inline and shows schedules only after I Have Reviewed This Result. Finish with Schedules Paused and Enable Schedules are separate actions. Closing the page keeps CLI progress; Cancel Setup pauses its imported routines and Resume Setup reuses its resources: [Workflow onboarding](workflows.md).

## Windows and Linux flow

The native Go/MyGo app adds Choose a Workflow… to the final onboarding page. The marketplace leads with Guided Workflows and includes packs in search; choosing a pack opens the native workflow sheet on the selected Runner. The outcome chooser also offers the paired Runner choices. The sheet names its fixed Runner throughout setup and opens the existing plugin sheet with the selected instance ID for sign-in and variables.

`desktop/model/workflows.go` decodes the additive pack/progress contract, including explicit service/instance IDs, specialists, sample replies and the CLI's ready/activation flags. It snapshots request parameters before leaving the main thread and uses `model.Async` and the ordered post queue for replies. The native view (`desktop/workflows.go`) keeps answers and specialist choices in persistent fields with stable question/role keys, so edits survive frames, roster updates and language changes. Replies after a sheet closes or after a newer request are ignored. Connection, roster and turn events refresh progress; editing keeps unsaved fields until Continue submits them.

The sheet presents only the required questions, explicit specialist/account menus and connection status. Missing or failed setup keeps its progress and offers refresh, retry, sign-in or clearing a removed account selection. A sample runs through `workflows.sample` and renders its replies before I Have Reviewed This Result. The reviewed result reveals Enable Schedules and Finish with Schedules Paused while imported routines remain paused. Cancel Setup pauses the owned routines and shows Resume Setup; closing the sheet retains CLI state for the next opening. The native app requests resource creation and activation through the CLI's existing grants, budget and admission checks.

`LORCA_MOCK=1` supplies synthetic pack/progress examples through the model's existing demo mechanism. The demo's transient presentation records contain no secrets and are separate from CLI execution. Native `ui.NewTester` tests click/type through the implemented sheets and can write offscreen MyGo frames with `LORCA_RENDER`; the captures use synthetic accounts and replies.
