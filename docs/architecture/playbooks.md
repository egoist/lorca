# Playbooks

A playbook is a user-authored reusable skill: instructions, examples, reference text, and optional script text. `crates/cli/src/playbooks.rs` owns the records; the AppKit and native Go/MyGo desktop apps read and edit them through their local CLI. Plugin-provided skills remain part of their installed plugin ([Plugins](plugins.md#plugins)); a user playbook has its own identity and revision history.

## Scope and contents

Every playbook has an immutable `Scope { kind, id }`. `bot` names an existing bot. `project` names an existing group chat: the project id is `ChatMeta.id`, membership is `ChatMeta.bot_ids`, and the owner remains `owner_bot_id`. A turn discovers its own bot's playbooks and the current group's project playbooks while the bot remains a member. A direct chat or routine discovers the bot scope; other groups' project playbooks stay in their own scope.

`PlaybookContent` carries `name` (a lowercase skill slug, at most 64 bytes), `description` (when to use it, at most 512 bytes), `instructions`, `examples`, `references`, and `scripts`. Each resource is an explicitly authored `{ path, text }` file under `references/` or `scripts/`; safe relative names have ASCII letters, numbers, hyphens, underscores, and dots. Absolute paths, dotfiles, empty path components, traversal, backslashes, and duplicate paths fail validation. The CLI stores the text supplied by the user or draft, with at most sixteen files in either section and 64 KiB of content per revision.

The reference and script editors edit bundled text. Saving or reading a script stores or returns that text; the bot executes it through its existing command tools and Auto-review boundary ([Tools](tools.md)). Skill prose also leaves the account's permission rules as they stand.

## Revisions and encrypted storage

The account's playbook library lives in the private `playbooks.enc` file, encrypted with the account DEK using XChaCha20-Poly1305 and `playbooks` as associated data. Writes use an atomic rename and mode 0600. The library travels inside the encrypted roster's optional `playbooks` field, so paired Devices hold it and the assigned Runner sees saved skills on later turns. The relay stores the roster ciphertext. Forgetting the account clears the local file and library.

A playbook has an opaque `playbook-…` id and immutable revisions. A revision records its id, parent id, revision number, `draft`, `saved`, or `deleted` status, content and SHA-256 content hash, Device id, timestamp, and provenance. Provenance names the source kind, optional source chat and selected message ids, and a note. The source chat belongs to the scoped bot or is the scoped project, and every cited message must exist there. Pattern scrubbing removes credentials from all authored text and provenance notes; exact secret values in the account's credentials are scrubbed too. The encrypted `credentials` blob and Runner plugin installs retain their own storage and lifecycle.

`playbooks.save` and `playbooks.remove` require both the displayed `expected_revision` and `expected_hash`. A stale editor gets a “changed since” error and retains its draft; it can reload the current revision before editing again. Save activates a draft and appends a revision. Removal appends a tombstone and makes the skill unavailable. The preceding content and provenance remain in history. A removed skill is recreated under a new id.

Sync merges revisions by playbook and revision id, preserves deletion records, and rejects a revision id whose content changes. Offline edits of one parent both survive; the largest revision number, then timestamp and revision id, select a deterministic current head. History shows the retained branches, and a guarded edit creates the next revision. A roster from a build without the field preserves the Device's library and queues a roster carrying it. The library is limited to 128 records, 128 revisions per record, and 2 MiB including history; a write exceeding a limit fails without discarding revisions.

## Discovery and lazy reads

`turns.rs` adds a user-playbook catalog through `playbook_tools.rs`. It lists saved names, descriptions, explicit scopes, ids, revisions, hashes, and `playbook://<id>/SKILL.md` paths within 6,000 bytes, with an omitted count. Drafts and deleted records stay out. Instructions, examples, and bundled files enter the model's context only through `read_playbook` after a match.

`list_playbooks { query?, sources? }` searches visible names and descriptions in a larger bounded result; `sources: true` additionally lists up to twenty completed user/current-bot text messages in the current chat with ids and short previews for capture. `read_playbook { path }` reads `SKILL.md`, generated with skill frontmatter and links to its bundled resources, or one exact resource path under the same `playbook://<id>/` directory. Each call resolves current scope and saved status again. Reading a project skill from a DM, another project, or after leaving the group fails.

## Workflow capture and standing instructions

Both desktop transcripts offer **Save Workflow as Skill…** on a completed bot reply and **Propose Standing Instructions…** on a completed user text message. The capture sheet shows the explicit destination scope and checkboxes for source messages. Workflow capture includes a completed reply by the selected bot; corrections capture requires at least two distinct completed user messages. The user selects related corrections and reviews the proposed rule. Group corrections target the group's owner, or its first member when it has no explicit owner; the scope picker offers that bot or the project.

`playbooks.draft { scope, chat_id, bot_id, kind, message_ids }` validates up to twenty selected messages, at most 24 KB of text. It excludes tool payloads, attachments, other bots, other chats, failed replies, and messages still streaming. Only the scrubbed selected text reaches the provider's small drafting model. The model has no tools; its strict content result is validated and saved as an inactive draft. The source is checked again after inference, and a timeout or invalid result leaves the library unchanged. Drafting usage counts in the source chat.

The bot's `propose_playbook` tool creates the same inactive draft using its authored content and validated evidence ids. A chat notice points the user to Playbooks. A proposal appears in the inspector's manager with **Draft** and opens the full editor for review. Only the user's **Save Skill** makes it available on later turns. Proposing or saving a standing instruction records provenance and revision history; it never adds an Auto-review rule or grants execution authority.

## App and API

The AppKit and native Go/MyGo inspectors' Playbooks section opens the manager for the current chat. A DM offers its bot; a group offers its project and each member bot explicitly. The manager fetches metadata, then loads one selected record's content and history for inspection or editing. It offers New Skill, Edit / Review, Export, Remove, and Reload. The editor has Name and Description, plus Instructions, Examples, References, Scripts, and History tabs. Reference and script files have their own name and text fields with Add File and Remove File. History displays the revisions, source notes, evidence ids, and preceding instructions.

The Go model (`desktop/model/playbooks.go`) decodes the same CLI records and sends every request through the store's ordered main-thread reply queue. The native sheets (`desktop/sheet_playbooks.go`) keep form text and file identities in persistent state; controls use stable keys before construction, including when a file is renamed or removed or the user changes tabs. Metadata requests have generations and explicit scopes, so a late reply from another selection is dropped. Dismissed sheets ignore outstanding replies. Save uses the opened revision and hash; a conflict keeps the draft and offers Reload, with no blind overwrite path. Remove uses the version the list displayed. Workflow/correction capture opens an inactive review editor after `playbooks.draft`; it sends `playbooks.save` only when the user presses Save Skill.

The local JSON methods are:

| Method | Parameters and result |
| --- | --- |
| `playbooks.list` | `{ scope, include_drafts? }` → `{ scope, items }`, metadata only |
| `playbooks.get` | `{ scope, id }` → current `content`, `revision`, `hash`, `status`, metadata, provenance, and `revisions` |
| `playbooks.save` | `{ scope, id?, content, expected_revision, expected_hash, provenance? }` → current record; new records use revision `0` and empty hash |
| `playbooks.remove` | `{ scope, id, expected_revision, expected_hash }` → the deletion revision |
| `playbooks.draft` | Explicit source selection as above → inactive record for review |
| `playbooks.export` | `{ scope, id }` → portable allowlisted content and files |

The core supports lifecycle and export on every Device. Draft generation uses a Runner's local CLI, where the provider and drafting model are available. The public Rust `get` and `save` adapters use the same scopes and guards as the JSON API.

## Portable export

Export selects one reviewed, saved playbook's current content. The result is version 1 of `lorca-playbook`: `{ format, version, content, files }`, where files contain generated `SKILL.md` and only that content's bundled references and scripts. The CLI scrubs the content again, preserving existing `«redacted credential»` and `«redacted N chars»` markers through repeated save/export/import cycles, and each desktop app writes only the returned JSON to the user's chosen file through its native save panel. The Go host runs MyGo's save dialog and the file write away from the UI build, then posts its result to the main thread. Scope ids, account and Runner identities, chat messages, memory, history, provenance, provider configuration, plugin configuration, and credentials stay outside the export. A receiving workflow creates an independent scoped playbook from `content` using a new id and the usual reviewed save; portable content grants no permissions.
