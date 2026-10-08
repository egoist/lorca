# Shared project context

A project is an existing group chat: its id is `ChatMeta.id`, its assigned bots are `ChatMeta.bot_ids`, and its owner is `owner_bot_id`. `crates/cli/src/project_context.rs` stores the group's shared brief, goals, constraints, decisions, facts, document links, and reference assets outside its transcript. A transcript compaction leaves this content intact. Each bot keeps its own [private memory](bots.md#memory).

## Scope and discovery

`project_for_turn(app, chat_id, bot_id)` selects only the current group and checks current membership. Every group turn receives a current context index: at most 24 entries and 8,000 UTF-8 bytes, including the explanatory text and omission count. Briefs, goals, constraints, and decisions come first. The index gives each revision's id, title, a short excerpt, source, update/verification/retrieval time, and freshness. The bot's `project_context` tool reads a complete entry on demand. A turn in a DM receives its private memory; it selects no project from another chat, the bot's working directory, or the recent-work brief.

Work associated with a group selects the same group when it starts a bot turn. The module owns context revisions, and uses the existing chat and membership records for project identity. An output provenance reference is `{ chat_id, message_id, output_id, version, task_id? }`: the chat must be this project, and the message/version names immutable supporting material. The optional task id is a reference to the task's own record. These references describe provenance; they do not establish that an output passed verification or a task completed.

## Entries and corrections

Each `Entry` contains `id` (`ctx-<UUID>`), `kind` (`brief`, `goal`, `constraint`, `decision`, `fact`, `document`, or `asset`), `title`, `text`, `source`, `verification`, `updated_at`, optional `verified_at`, `fetched_at`, and `max_age_secs`, `supersedes`, `removed`, optional `asset`, and `refresh_error`. A source has `kind` (`user`, `bot`, `url`, `message`, or `output`), a label, and its URL, message id, or output-version reference when applicable. The CLI stamps update times; a user's explicit agreement or verification stamps `verified_at`. Text and source labels pass through the memory credential scrubber. An entry carries at most 32,000 bytes of text, and local writes keep the project history within 8 MiB.

Revisions are immutable. A correction names the current revisions it supersedes and the `expected_revision` hash returned by `projects.get`; a stale editor reloads and reapplies its change. The old text and provenance remain in history. Removal creates a revision with `removed: true` that supersedes the selected entries. A replay of an existing id changes nothing, and different content under that id is refused.

Separate Devices can correct the same predecessor before receiving each other's changes. Both successor revisions remain current, and `projects.get.conflicts` names the branches. A user resolves them by superseding both, or removing the unwanted branch. No whole-project last-writer snapshot drops the other correction. The roster supplies group membership; deleting the group deletes its local context and queued blobs and uses the existing relay group deletion to remove context and assets there.

## Sources and freshness

Verification states are `agreed` (accepted by the user), `verified` (explicitly checked), `fetched` (retrieved source content), `unverified`, and `unavailable`. Retrieving a document does not agree to its statements. The index marks a live URL `stale` when its newest retrieval or verification is older than `max_age_secs` (one day by default), and marks a URL with no check time `unverified`.

For work needing current facts, the prompt instructs the bot to refresh the referenced source before relying on it. `project_context { action: "refresh", entry_id }`, or `read` with `fresh: true`, performs a bounded HTTPS fetch. `projects.refresh` exposes the same operation to the apps. The fetch takes at most 20 seconds and 32,000 bytes, accepts UTF-8 text, HTML, JSON, or XML, and uses no provider or integration credentials. A redirect asks for the final HTTPS URL. Binary documents travel as reference assets. Authenticated integrations use their existing Runner tools; the project stores their supplied evidence and provenance.

Success creates a `fetched` revision with `fetched_at`, clears `verified_at`, and keeps its predecessor in history. Failure creates an `unavailable` revision, retains the prior source content and last successful retrieval time, and records why. A refresh that completes after a correction refuses to replace it. Agreed decisions are corrected by the user, with their document or factual sources refreshed separately. Source content is labelled as data in the prompt and tool results. The bot's `propose` action adds an unverified candidate for the user's review and supersedes no agreed entry.

## Storage and encrypted sync

The local SQLite `project_entries` table has `(chat_id, id, ciphertext)`. The authenticated plaintext inside each ciphertext is `ProjectBlob { chat_id, entry }`; reading it checks the inner scope against the requested group. The account DEK encrypts it with associated data `project_context`. A local revision and its encrypted outbox item commit in one transaction. Every paired Device pulls the `project_context` blob kind, one opaque relay slot per immutable revision, grouped by the existing chat id. Protocol 3 supplies this kind.

First sync installs the latest roster before pulling project revisions, then pulls all account metadata and project revisions before the initial transcript pages. A brief remains discoverable after thousands of chat messages. A Device upgrading from a build that did not poll this kind backfills project revisions once after its current roster lands; SQLite records the completed pull for that Device so reconnects keep the regular log cursor. A relay recovery uploads the locally retained revisions again. The relay handles these as opaque blobs with the existing quotas, slots, and group deletion; it sees no titles, facts, or sources.

Reference assets reuse `files::store`, encrypted `file` blobs, and `files::ensure_local`. Each project attachment gets a fresh attachment id, even when the caller supplies one, so different projects never reuse a relay file id. The encrypted entry holds its name, MIME, size, and dimensions. Bytes are fetched on demand; the app sees an explicit unavailable error and can retry. A bot's asset read copies the file beneath `<workdir>/projects/<group>/attachments/<id>/<name>`, using safe group and file names. Device-local copies follow the private permissions of the existing attachment store.

## Local API and desktop editors

- `projects.get { chat_id, entry_id?, history?, after?, limit? }` returns `revision`, paged `entries` (at most 100), `has_more`, `conflicts`, and the prompt budget. Each entry adds `current`, computed `freshness`, and an asset's local availability. `history: true` includes predecessors and removals.
- `projects.save { chat_id, kind, title, text?, source?, verification?, max_age_secs?, supersedes?, removed?, expected_revision? }` appends a revision. Corrections and removals require the current revision hash. Asset corrections reuse the asset of their scoped predecessor.
- `projects.refresh { chat_id, entry_id }` rechecks a current URL source.
- `projects.asset { chat_id, file: { path, name?, mime?, width?, height? }, title?, text? }` stores and encrypts a reference file.
- `projects.asset_path { chat_id, entry_id }` fetches a current asset and returns its local path.

A committed revision emits `projects.changed { chat_id, entry_id }`. The bot's `project_context` tool binds its group and bot in the runtime; its arguments cannot select another project, and membership is checked again after an asynchronous read.

The AppKit group inspector has a Project context row opening `ProjectContextViewController`. The sheet lists current entries or revision history, edits type/title/content/source/status, displays update and verification times, refreshes live sources, adds and opens reference files, and removes or corrects current entries with revision checks. Reload retrieves the latest version; a conflict leaves the user's draft available to reapply. Every operation goes through the local CLI.

The native Windows/Linux Go/MyGo group inspector opens `desktop/sheet_project_context.go` from the same Project context row. `desktop/model/project_context.go` decodes the CLI's entries, source verification/retrieval times, immutable message/output provenance, correction hashes, conflicts, and reference-file metadata. It pages `projects.get` within one group and revision, refusing a mixed-scope or changing paginated result. Save, source refresh, asset add, and asset retrieval call the existing local CLI methods; the desktop stores no project records or account secrets of its own.

The sheet keeps draft fields and the selected entry outside the build pass, with stable group/entry/field keys. A current entry can be corrected or removed; historical revisions are selectable and have Save and Remove disabled. Source refresh displays `unavailable` and the retained snapshot when the CLI records a fetch failure. The asset action asks `projects.asset_path`, shows an unavailable error for a retry, and opens a returned path through the existing host file opener. The file picker sends the chosen local path to `projects.asset` for scoped encrypted storage.

CLI answers return through the store's ordered main-thread post queue. Load generations reject stale replies, and dismissed sheets stop observing events and ignore pending responses. A `projects.changed` event reloads a clean current view and marks an edited draft out of date without replacing it. A stale correction likewise retains the draft and offers Reload; changing entries, reloading, closing, or refreshing with unsaved changes asks before discarding them. Group deletion disables the editor. Date labels and verification/freshness words use the desktop's current language.
