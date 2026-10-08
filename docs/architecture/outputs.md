# Outputs and evidence

A bot publishes a generated file or an existing HTTPS document link with `publish_output`. An output is a completed text message with additive `Message.output` metadata; its file stays in the message's `attachments`. The producing bot and chat come from the turn's bound context. Every paired Device reads the same record through encrypted chat sync, and clients that render text and attachments show the deliverable too.

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

Exactly one of `path` and `url` is required. A relative path resolves in the producing bot's working directory; `mime` overrides the filename's media type. Publication runs on that bot's assigned Runner and requires membership in the chat and an account key. Files are regular files up to 100 MiB, copied into the private attachment store with a fresh attachment id. The file blob is encrypted with the account DEK and queued before the chat message. The source path stays on the Runner. A changed or removed source file does not change the published copy.

A `url` is an HTTPS document reference without embedded credentials. Recording it performs no request to the document's service. Creating a document, uploading bytes, or publishing externally uses the existing shell or plugin tools and their [Auto-review boundary](tools.md). Integrations and their credentials stay on the assigned Runner.

The local API is `outputs.publish { chat_id, bot_id, ...arguments }`, on the producing Runner, and `outputs.list { chat_id, task_id? }`, on any paired Device. Publish returns `{ message, task_evidence }`. List returns `{ outputs: [Message], has_more }` in transcript order: every locally synced version, with `has_more` when earlier chat history remains on the relay. `chats.messages` loads earlier history. A SQL query selects output rows without materializing unrelated tool results.

## Versions and references

`Message.output` carries `id`, `name`, `mime`, `bot_id`, `chat_id`, optional `task_id`, `version`, optional `previous_message_id`, optional `url`, and optional `evidence`. The stable `out-UUID` id identifies a deliverable's series. Each version has its own immutable message id and, for files, its own attachment id.

Passing `replaces: <previous message id>` publishes a new version. It keeps the series id, producing bot, chat and task, increments `version`, and retains the preceding message and bytes. Publication is serialized on the producing Runner; a stale predecessor or a predecessor from another scope is refused. The encrypted outbox and relay have separate message slots for every version, so superseding a streamed message does not discard output history. Chat deletion uses the existing group deletion to remove its output messages and file blobs together.

The shared reference is `{ chat_id, message_id, output_id, version }` (`OutputReference`). `Output::task_evidence(message_id)` supplies `{ kind: "output", label, ...reference }`, an immutable task evidence reference. `task_id` uses canonical `task-UUID` ids. Outputs store references to tasks and introduce no task records. The bot tool returns the reference as text and as structured codemode output, alongside its metadata.

## Verification evidence

`evidence` carries `kind`, `summary`, `status`, optional `command`, and optional `exit_code`. Kinds are `test_result`, `before_screenshot`, `after_screenshot`, and `verification`; statuses are `passed`, `failed`, and `unverified`. A nonzero exit code conflicts with `passed`; zero conflicts with `failed`. A local screenshot requires an image with PNG, JPEG, GIF, or WebP dimensions. Tests and visual work retain their logs, reports, and before/after files as output versions. The turn prompt directs bots to publish this evidence and describe remaining uncertainty.

Evidence reports the producing bot's verification claim; storing a result does not execute a check or mark a task complete. The record retains its canonical task reference and immutable output version; task completion is a separate operation. Later turns fetch published file attachments and materialize them into the reading bot's workspace, with provenance and version ids in model context, so a teammate can inspect the underlying result.

## Retrieving and inspecting

`files.path { attachment, named?: true }` fetches missing bytes from the relay, opens them under the account DEK, and returns a local path. The optional named copy is private and keeps the original extension under `files/open/<attachment id>/<safe name>`, so AppKit's handlers recognize reports and recordings. Attachment ids are checked before resolving paths. An absent relay blob returns a clear unavailable-file error; a user can retry after connectivity or an upload recovers.

The [macOS Outputs sheet](macos-app.md) lists all synced versions with the producing bot, task reference and verification summary. Image thumbnails appear with the text in the transcript and result surface. Preview uses Quick Look for images, PDFs, reports and recordings; Open uses the system's file handler, Save As copies to the user's selected destination, and Open document follows an HTTPS link. Fetch failures display the error and Retry rather than keeping an indefinite fetching state. The app obtains every file through the local CLI.

The [Windows and Linux app](desktop-app.md) reads the same message metadata and APIs in `desktop/model/outputs.go`. Its native MyGo Outputs sheet is available from the chat header, Chat menu and command palette, with keyed immutable version/evidence cards and older-history loading. Local PNG/JPEG/GIF/WebP and other supported images, UTF-8 reports/logs/JSON/CSV, and bounded text previews use native views; Open delegates PDFs and recordings to the system app. Save As downloads the CLI-retrieved file through the native destination dialog, preserving existing files on a failed copy. Named local retrieval, unavailable reasons and Retry also apply to transcript attachment tiles. An existing HTTPS document link opens only on explicit action. The UI keeps request generations and transient sheet state on the main thread; a closed sheet or older reply cannot replace current results.
