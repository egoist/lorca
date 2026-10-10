# Outputs and evidence

A bot publishes a generated file or an existing HTTPS document link with `publish_output`. An output is a completed text message with additive `Message.output` metadata; its file stays in the message's `attachments`, and its text says what the transcript shows. The producing bot and chat come from the turn's bound context. Every paired Device reads the same record through encrypted chat sync, and clients that render text and attachments show the deliverable too.

## Publishing

`crates/cli/src/outputs.rs` owns the publication helper, output metadata, evidence, immutable references, and the bot tool. The tool is available directly and in codemode as `tools.publish_output`. Its arguments are:

```json
{
  "name": "Verification.txt",
  "path": "results/tests.txt",
  "task_id": "task-3a249410-8034-4be2-bf42-9e68b4fe2c41",
  "evidence": {
    "kind": "test_result",
    "summary": "The focused tests pass.",
    "status": "passed",
    "command": "cargo test -p lorca outputs::tests --lib",
    "exit_code": 0
  }
}
```

Exactly one of `path` and `url` is required. A relative path resolves in the producing bot's working directory; `mime` overrides the filename's media type. Publication runs on that bot's assigned Runner and requires membership in the chat and an account key. A file also needs the bot's [Access](bot-permissions.md#execution-boundary) to allow reading files; without it the call is refused and the user is asked, as for `read`. Files are regular files up to 100 MiB, copied into the private attachment store with a fresh attachment id. The file blob is encrypted with the account DEK and queued before the chat message. The source path stays on the Runner. A changed or removed source file does not change the published copy.

A `url` is an HTTPS document reference without embedded credentials. Recording it performs no request to the document's service. Creating a document, uploading bytes, or publishing externally uses the existing shell or plugin tools and their [Auto-review boundary](tools.md). Integrations and their credentials stay on the assigned Runner.

The local API is `outputs.publish { chat_id, bot_id, ...arguments }`, on the producing Runner, and `outputs.list { chat_id, task_id? }`, on any paired Device. Publish returns `{ message, task_evidence }`. List returns `{ outputs: [Message] }` in transcript order: every version this Device has synced. A SQL query selects output rows without materializing unrelated tool results.

## Versions and references

`Message.output` carries `id`, `name`, `mime`, `bot_id`, `chat_id`, optional `task_id`, `version`, optional `previous_message_id`, optional `url`, and optional `evidence`. The stable `out-UUID` id identifies a deliverable's series. Each version has its own immutable message id and, for files, its own attachment id.

Passing `replaces: <previous message id>` publishes a new version. It keeps the series id, producing bot, chat and task, increments `version`, and retains the preceding message and bytes. Publication is serialized on the producing Runner; a stale predecessor or a predecessor from another scope is refused. The encrypted outbox and relay have separate message slots for every version, so superseding a streamed message does not discard output history. Chat deletion uses the existing group deletion to remove its output messages and file blobs together.

The shared reference is `{ chat_id, message_id, output_id, version }` (`OutputReference`). `Output::task_evidence(message_id)` supplies `{ kind: "output", label, ...reference }`, an immutable task evidence reference. `task_id` uses canonical `task-UUID` ids. Outputs store references to tasks and introduce no task records. The bot tool returns the reference as text and as structured codemode output, alongside its metadata.

## Verification evidence

`evidence` carries `kind`, `summary`, `status`, optional `command`, and optional `exit_code`. Kinds are `test_result`, `before_screenshot`, `after_screenshot`, and `verification`; statuses are `passed`, `failed`, and `unverified`. A nonzero exit code conflicts with `passed`; zero conflicts with `failed`. A local screenshot requires an image with PNG, JPEG, GIF, or WebP dimensions. Tests and visual work retain their logs, reports, and before/after files as output versions. The turn prompt directs bots to publish this evidence and describe remaining uncertainty.

A [coding agent](coding-agents.md#when-it-is-done) a bot runs leaves its work the same way, as its bot's outputs each time it is done: its diff, the pull request it names, and the files it saved as proof, its evidence `unverified`.

Evidence reports the producing bot's verification claim; storing a result does not execute a check or mark a task complete. The record retains its canonical task reference and immutable output version; task completion is a separate operation. Later turns fetch published file attachments and materialize them into the reading bot's workspace, with provenance and version ids in model context, so a teammate can inspect the underlying result.

## Retrieving and inspecting

`files.path { attachment, named?: true }` fetches missing bytes from the relay, opens them under the account DEK, and returns a local path. The optional named copy is private and keeps the original extension under `files/open/<attachment id>/<safe name>`, so AppKit's handlers recognize reports and recordings. Attachment ids are checked before resolving paths. An absent relay blob returns a clear unavailable-file error; a user can retry after connectivity or an upload recovers.

Every app shows an output in the transcript as the message it is: its file's tile, and its text, which the CLI writes in English from the output (`outputs::message_text`): the link, and what the bot checked (`Test result · Passed: All 23 links answer.` with the command as inline code). A file alone has no text; the bot and the version are the message's own. The macOS app and the Windows and Linux app also list a chat's outputs in the inspector, the latest three with View all, and open each in a sheet with a preview, the check, every version, Open, and Save As… ([macOS chat](macos-chat.md#outputs), [Windows and Linux app](desktop-app.md)); the phone lists them in a chat's Details and opens each in a screen with Open and Share… ([Phone app](phone-app.md)), fetching the file through its own core like any other Device. They get every file through the local CLI, ask for the named copy only to open or save it, and keep a failed fetch's reason until the user retries. An HTTPS document link opens only when the user opens it.
