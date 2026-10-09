# Shared project context

A project is an existing group chat: its id is `ChatMeta.id`, its assigned bots are `ChatMeta.bot_ids`, and its owner is `owner_bot_id`. `crates/cli/src/project_context.rs` stores the group's shared brief, goals, constraints, decisions, facts, document links, and reference assets outside its transcript. A transcript compaction leaves this content intact. Each bot keeps its own [private memory](bots.md#memory).

## Scope and discovery

`project_for_turn(app, chat_id, bot_id)` selects only the current group and checks current membership. A group turn receives the group's current context index, and nothing while the group has none: at most 24 entries and 8,000 UTF-8 bytes, including the explanatory text and omission count. Briefs, goals, constraints, and decisions come first. The index gives each revision's id, title, a short excerpt, source, update/verification/retrieval time, and freshness. The bot's `project_context` tool reads a complete entry on demand. A turn in a DM receives its private memory; it selects no project from another chat, the bot's working directory, or the recent-work brief.

A task run or a handoff in a group is a turn in that group, so it reads the group's context; one in a DM reads none. The module owns context revisions, and uses the existing chat and membership records for project identity. An output source cites one version a bot published in the group, as the shared `outputs::OutputReference` `{ chat_id, message_id, output_id, version }`; saving a new citation checks it against that message's output metadata, and a correction that keeps its predecessor's citation needs no message this Device may lack. The output's own task is the task it belongs to. A citation describes provenance; it does not establish that an output passed verification or a task completed.

## Entries and corrections

Each `Entry` contains `id` (`ctx-<UUID>`), `kind` (`brief`, `goal`, `constraint`, `decision`, `fact`, `document`, or `asset`), `title`, `text`, `source`, `verification`, `updated_at`, optional `verified_at`, `fetched_at`, and `max_age_secs`, `supersedes`, `removed`, optional `asset`, and `refresh_error`. A source has `kind` (`user`, `bot`, `url`, `message`, or `output`), a label, and its URL, message id, or output-version reference when applicable. The CLI stamps update times; a user's explicit agreement or verification stamps `verified_at`. An entry the user writes has the source `user`; a bot's proposal has the source `bot` with the bot's name as its label. Text and source labels pass through the memory credential scrubber. An entry carries at most 32,000 bytes of text, and local writes keep the project history within 8 MiB.

Revisions are immutable. A correction names the current revisions it supersedes. When another Device corrected or removed one of them first, the save is refused with "This entry changed on another Device.", so neither change is lost; other entries changing meanwhile hold nothing up. The old text and provenance remain in history. Removal creates a revision with `removed: true` that supersedes the selected entries. A replay of an existing id changes nothing, and different content under that id is refused.

Separate Devices can correct the same predecessor before receiving each other's changes. Both successor revisions remain current, and `projects.get.conflicts` names the branches. A user resolves them by superseding both, or removing the unwanted branch. No whole-project last-writer snapshot drops the other correction. The roster supplies group membership; deleting the group deletes its local context and queued blobs and uses the existing relay group deletion to remove context and assets there.

## Sources and freshness

Verification states are `agreed` (accepted by the user), `verified` (explicitly checked), `fetched` (retrieved source content), `unverified`, and `unavailable`. Retrieving a document does not agree to its statements. The index marks a live URL `stale` when its newest retrieval or verification is older than `max_age_secs` (one day by default), and marks a URL with no check time `unverified`.

For work needing current facts, the prompt instructs the bot to refresh the referenced source before relying on it. `project_context { action: "refresh", entry_id }`, or `read` with `fresh: true`, performs a bounded HTTPS fetch. `projects.refresh` exposes the same operation to the apps. The fetch takes at most 20 seconds and 32,000 bytes, accepts UTF-8 text, HTML, JSON, or XML, and uses no provider or integration credentials. A redirect asks for the final HTTPS URL. Binary documents travel as reference assets. Authenticated integrations use their existing Runner tools; the project stores their supplied evidence and provenance.

Success creates a `fetched` revision with `fetched_at`, clears `verified_at`, and keeps its predecessor in history. Failure creates an `unavailable` revision, retains the prior source content and last successful retrieval time, and records why. A refresh that completes after a correction refuses to replace it. Agreed decisions are corrected by the user, with their document or factual sources refreshed separately. Source content is labelled as data in the prompt and tool results. The bot's `propose` action adds an unverified candidate for the user's review and supersedes no agreed entry.

## Storage and encrypted sync

The local SQLite `project_entries` table has `(chat_id, id, ciphertext)`. The authenticated plaintext inside each ciphertext is `ProjectBlob { chat_id, entry }`; reading it checks the inner scope against the requested group. The account DEK encrypts it with associated data `project_context`. A local revision and its encrypted outbox item commit in one transaction. Every paired Device pulls the `project_context` blob kind, one opaque relay slot per immutable revision, grouped by the existing chat id. Protocol 3 supplies this kind.

First sync pulls every project revision with the account metadata, before the initial transcript pages, so a brief remains discoverable after thousands of chat messages. The relay keeps only the latest roster, which can come after the context of a group it lists, so a Device keeps a revision whose group has not landed yet; rows of a group that never arrives go with the other chats' leftovers at the next launch. A relay recovery uploads the locally retained revisions again. The relay handles these as opaque blobs with the existing quotas, slots, and group deletion; it sees no titles, facts, or sources.

Reference assets reuse `files::store`, encrypted `file` blobs, and `files::ensure_local`. Each project attachment gets a fresh attachment id, even when the caller supplies one, so different projects never reuse a relay file id. The encrypted entry holds its name, MIME, size, and dimensions. Bytes are fetched on demand; the app sees an explicit unavailable error and can retry. A bot's asset read copies the file beneath `<workdir>/projects/<group>/attachments/<id>/<name>`, using safe group and file names. Device-local copies follow the private permissions of the existing attachment store.

## Local API

- `projects.get { chat_id, entry_id?, history?, after?, limit? }` returns paged `entries` (at most 100, by id), `has_more`, and `conflicts`. Each entry adds `current`, computed `freshness`, and an asset's local availability. `history: true` includes predecessors and removals.
- `projects.save { chat_id, kind, title, text?, source?, verification?, max_age_secs?, supersedes?, removed? }` appends a revision. Asset corrections reuse the asset of their scoped predecessor.
- `projects.refresh { chat_id, entry_id }` rechecks a current URL source.
- `projects.asset { chat_id, file: { path, name?, mime?, width?, height? }, title?, text? }` stores and encrypts a reference file.
- `projects.asset_path { chat_id, entry_id }` fetches a current asset and returns its local path.

A committed revision emits `projects.changed { chat_id, entry_id }`. The bot's `project_context` tool binds its group and bot in the runtime; its arguments cannot select another project, and membership is checked again after an asynchronous read.

## The apps

A group's inspector has a Project section under Group, on the Mac and in the Windows and Linux app, and a group's Details on the phone has the same section ([Phone app](phone-app.md)). It lists the current entries a row each, briefs, goals, constraints, decisions, facts, links, then files, newest first within a kind; past five rows, four show with Show N More. A row says its kind and its link's host, or "Suggested by <bot>" for a bot's proposal; two versions of one entry and a link that could not be opened carry an orange mark and say so. With no entries the section is its title alone. The title's + menu adds a brief, goal, constraint, decision, fact, link, or file (picked in an open panel and sent to `projects.asset`).

A row opens the entry's sheet (`ProjectEntryViewController` on the Mac): its title, text, and source link, with a link's address first and a file shown with Open (`projects.asset_path`, then the system opener). The subtitle says who saved it and when, or which host it came from; under a link, when it was last checked, or that it could not be opened, and Check Now (`projects.refresh`), which an agreed decision does not offer. Save sends a correction the user agrees to. A bot's suggestion reads Accept, and an entry with two versions reads Keep This Version and replaces the other. Remove… asks first. When another Device changed the entry first, the sheet offers that version or to overwrite it with the edits, as the memory editor does, and a removed entry can be added back. The section fetches again on `projects.changed`. Revision history stays with the CLI and the bots' `project_context` reads; the apps show the current entries.

The Windows and Linux app draws the same section and sheet (`desktop/project.go`), with its CLI calls in `desktop/model/project_context.go`; their replies come back on the store's main-thread queue, and a sheet that was closed meanwhile ignores them.
