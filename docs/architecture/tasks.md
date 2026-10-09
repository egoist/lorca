# Durable tasks

`crates/cli/src/tasks/` owns work that spans Jobs, chats, compaction, and Runner restarts. A Job is one bot turn; a Task retains the goal, owner, definition of done, blocker, and next step between turns. Tasks live independently of the roster and transcript.

## Records and states

Each Task has an opaque `task-<UUID>` id, increasing `revision`, `owner_bot_id`, assigned `runner_id`, `goal`, nonempty `acceptance_criteria`, `dependencies` naming other task ids, `next_action`, linked `chat_ids`, HTTPS external `links`, `state`, optional `reason` and `result`, supporting `evidence`, optional `active_run`, and creation/update times. The owning bot runs on the assigned Runner and belongs to at least one linked chat in which it executes. Other linked chats carry teammate work and evidence references. A group's chat owner and a task's owner are independent.

States are `queued`, `working`, `blocked`, `awaiting_review`, `completed`, and `cancelled`. Blocked and cancelled require a reason. Completion requires a result and at least one supporting evidence reference. Every committed edit records its revision, owner, Runner, state, reason, and time in `history`. A terminal task or one awaiting review is explicitly requeued before another run. Creation queues work; only `tasks.run` sets `working`.

Dependencies exist and form an acyclic graph. A run starts only when every direct dependency is completed. Queuing runs no model or tool, and a completed dependency does not automatically start its dependents.

`TaskEvidence` carries `kind`, a human-readable `label`, and reference fields:

| Kind | Reference |
| --- | --- |
| `message` | `chat_id`, `message_id` |
| `file` | `chat_id`, `message_id`, `attachment_id` from that message |
| `url` | An HTTPS `url` |
| `output` | `chat_id`, `message_id`, `output_id`, `version`; the immutable message's output metadata names the same task, output series, and version |
| `review` | `review_id`, `chat_id`, the review's `review-status-<review id>` outcome `message_id` |

Message references resolve through `App::message` and stay within linked chats. An edit resolves only the references it adds; completing a task resolves every one. An output reference checks the serialized message's `output.id`, `output.version`, `output.task_id`, and `output.chat_id`. Evidence stores references; attachment bytes use the encrypted file store. Adding a reference does not approve an action or complete a task. A remote run result can reach the authority before its chat blobs: the awaiting-review record keeps that scoped reference, and a completion edit resolves its message before succeeding.

## Authority and persistence

The initial owner's Runner becomes `authority_runner_id`. It remains the authority after an ownership transfer. Writes route to it through the existing sealed [request/response protocol](protocols.md#cli--relay). An unavailable authority refuses edits explicitly; paired Devices inspect their persisted replicas. Integrations and execution stay on the assigned Runner.

Each mutation carries `request_id`; updates and runs also carry `expected_revision`. The authority serializes validation and uses a SQLite compare-and-swap. A stale ownership or progress edit fails with the current revision instead of replacing newer work. The encrypted receipt binds the method, complete parameters, and original reply to the request id. An identical retry returns that reply without another write or run; reusing an id with different parameters fails. The record, receipt, encrypted sync outbox entry, and run dispatch intent commit in one transaction. Receipts survive restarts. Task mutations and execution claims use full SQLite WAL synchronization before acknowledgement or tool effects, then restore the database's normal synchronization level.

Task content, receipts, and run journals are account-DEK ciphertext in `lorca.sqlite3`, with associated-data kinds `task`, `task-receipt`, `task-run`, and `task-finish`. An encrypted relay `task` blob carries one record in its own latest-only slot, outside chat groups and the roster. A newer revision advances a replica; older delivery is a no-op; different authority or content at the same revision is a logged sync conflict. The authority refuses an incoming revision it has not committed. First sync includes tasks, and the authority requeues its records when uploading account history again. The relay sees ciphertext and opaque slots.

## Execution and recovery

`tasks.run` assigns a fresh `task-run-<UUID>` Job id and records `working` with `active_run`. A `kind = task` Job executes on the assigned Runner. `Job.task_id` names the task and `task_context` carries the authority's dispatch snapshot, so a sealed Job can arrive before task sync. The run's bot, Runner, and chat match its task. Other Job kinds can reference a task without claiming or finishing its active run.

The assigned Runner persists an execution claim before registering the Job or running tools. Repeated delivery cannot replace the live cancellation token or execute the Job again. After taking the chat lock, the Runner revalidates task/run ownership, bot assignment, and chat; another Runner asks the authority to validate its run. Ownership, scope, acceptance criteria, and dependencies cannot change during an active run. Changing a working run to another state sends a job cancellation; the active run stays claimed until its outcome clears it. A run that records its own outcome through the `tasks` tool ends its turn without one.

A successful turn records its final completed text reply and message evidence, plus the newest version of each [output](outputs.md) it published for the task (`Output::task_evidence`, the same reference `publish_output` returns), and moves a still-working task to `awaiting_review`. A reply alone does not complete work. A stopped, failed, or empty turn records `blocked` with a reason. A state the bot or user explicitly records during the run remains when the run finishes. When an optional budget snapshot marks this Runner's task scope exhausted or interrupted, its reason blocks the task even after a partial reply. Budget recovery renews or adjusts the task scope without replaying its saved Job, then starts a fresh task run. Outcomes are journaled before sending to the authority, retried while it is unavailable, and applied by run id with a durable receipt.

On restart, unclaimed local dispatch resumes. A claimed run becomes interrupted with a reason saying its effects may have occurred; the old execution id stays claimed. Recovery requires inspecting those effects and explicitly starting a new run. The fixed authority's machine key remains the mutation address; restoring the account on another machine does not transfer that authority automatically.

## Bots

The bot tool `tasks` has `list`, `get`, `create`, `update`, and `run` actions matching the API. Creation defaults to the calling bot and chat; listing defaults to the current chat, while `all: true` or an explicit owner filter spans chats. The tool's get refreshes from the authority by default and takes `refresh: false` for a local replica. Mutation ids stay explicit so an ambiguous delivery can be retried with identical arguments.

A bot with open tasks (owned by it or linked to the chat, not completed or cancelled) reads a fresh note after the transcript on every provider request, through turn hooks, even after mid-turn compaction; a bot with none gets no note. It includes owners, revisions, criteria, dependencies, state, next action, result, and evidence. The note is not a chat message or compaction summary. It includes up to twenty open records and 32 KB and directs the bot to list/get for the rest. Task data is context; shell and integration actions retain their existing review.

## In the apps

The Mac app and the Windows and Linux app show a chat's tasks in the inspector's Tasks section, which hides while the chat has none (the phone's Details has the same section, with New Task as its last row; see [Phone app](phone-app.md)): blocked and awaiting-review tasks first, then working, queued, and finished ones, newest first, with five rows before Show More. A row shows the goal, the state, and in a group the owner, and opens the task's sheet. The section title's + and File › New Task… create a task for the chat on screen: its goal, owner (a bot in the chat), next step, and acceptance criteria, one per line.

The sheet's first card shows the state, the reason for a blocked or cancelled task (not the app's own "Cancelled by the user."), and the result with its evidence: a message by who wrote it and when, an output by its name and version, which opens that version, a link by its host, which opens it. The card offers the one step the state allows: Start or Resume (`tasks.run`, in this chat when the owner belongs to it, else the first linked chat it does), Mark Complete for a result with evidence, Reopen for a finished task (`state: queued`). Cancel Task… asks first, then records `cancelled`. The tasks a task waits for and its external links follow, read-only; Start waits until the dependencies are completed.

The goal, owner, next step, and acceptance criteria are a form, locked to edits of ownership and criteria during a run. Save sends the fields that changed with the revision the form started from; a step sends the form's edits along with it. A change from elsewhere replaces a form the user has not touched. When a save meets a newer revision, the sheet reads the task again, keeps each field the user changed over the newer version's, says that the task changed, and the next Save writes the edits. A request retried after an unclear failure keeps its `request_id`. All reads and writes go through the local CLI.

## API and terminal

Bootstrap includes `tasks`; `tasks.changed { task }` publishes persisted changes.

- `tasks.list { chat_id?, owner_bot_id?, state? }` returns locally synced records.
- `tasks.get { id, refresh? }` reads one record; `refresh: true` asks a remote authority.
- `tasks.create { id?, request_id, owner_bot_id, goal, acceptance_criteria, next_action, chat_ids, dependencies?, links? }` queues a task on the owner's Runner.
- `tasks.update { id, expected_revision, request_id, ...fields }` changes supplied editable fields. Null clears `reason` or `result`. Evidence is the complete replacement list.
- `tasks.run { id, expected_revision, request_id, chat_id? }` starts one claimed turn, in a linked chat the owner belongs to (the first eligible chat by default).

`lorca tasks list` takes `--chat-id`, `--owner-bot-id`, and `--state`; `lorca tasks get <id>` reads one record. `lorca tasks create '<JSON>'`, `update '<JSON>'`, and `run '<JSON>'` send their parameter object to `lorca serve`; `-` reads JSON from stdin. Mutations require the service. Standalone reads inspect the local database.
