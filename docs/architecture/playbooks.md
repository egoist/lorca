# Playbooks

A playbook is a skill the user writes for a bot or a group: instructions, an example, reference text, and optional script text. The apps call them skills. `crates/cli/src/playbooks.rs` owns the records; the macOS app and the Windows and Linux app read and edit them through their local CLI. Plugin-provided skills remain part of their installed plugin ([Plugins](plugins.md#plugins)); a user playbook has its own identity and revision history.

## Scope and contents

Every playbook has an immutable `Scope { kind, id }`. `bot` names an existing bot. `project` names an existing group chat: the project id is `ChatMeta.id`, membership is `ChatMeta.bot_ids`, and the owner remains `owner_bot_id`. A turn discovers its own bot's playbooks and the current group's project playbooks while the bot remains a member. A direct chat or routine discovers the bot scope; other groups' project playbooks stay in their own scope.

`PlaybookContent` carries `name` (a lowercase slug, at most 64 bytes), `description` (when to use it, at most 512 bytes), `instructions`, `examples`, `references`, and `scripts`. Each resource is an explicitly authored `{ path, text }` file under `references/` or `scripts/`; safe relative names have ASCII letters, numbers, hyphens, underscores, and dots. Absolute paths, dotfiles, empty path components, traversal, backslashes, and duplicate paths fail validation. The CLI stores the text supplied by the user or a draft, with at most sixteen files in either folder and 64 KiB of content per revision.

Reference and script files are text the skill holds. Saving or reading a script stores or returns that text; the bot runs it through its existing command tools and Auto-review ([Tools](tools.md)). Skill prose leaves the account's permission rules as they stand.

## Revisions and encrypted storage

The account's playbook library lives in the private `playbooks.enc` file, encrypted with the account DEK using XChaCha20-Poly1305 and `playbooks` as associated data. Writes use an atomic rename and mode 0600. Each skill syncs as a `playbook` blob of its own: its whole record, revisions included, encrypted with the account DEK (`playbook` as associated data) in a latest-only slot named by its id, sent whenever it changes, so paired Devices hold it and the assigned Runner sees saved skills on later turns. The roster carries no skills. A skill's blob belongs to the group of its scope's chat, the group's own or the bot's DM, so deleting that chat deletes the blobs on the relay. When `playbooks.enc` cannot be read, the CLI starts with an empty library, which the other Devices' blobs fill again. Forgetting the account clears the file and the library.

A playbook has an opaque `playbook-…` id and immutable revisions. A revision records its id, parent id, revision number, `draft`, `saved`, or `deleted` status, content and SHA-256 content hash, Device id, timestamp, and provenance. Provenance names the source kind (`manual`, `workflow`, `corrections`, `edit`, `remove`), optional source chat and selected message ids, and a note. A cited source chat belongs to the scoped bot or is the scoped project, and every cited message must exist there; an edit in the apps cites none, so a skill whose source chat is gone stays editable. Pattern scrubbing removes credentials from all authored text and provenance notes as a revision is written; exact secret values in the account's credentials are scrubbed too. A stored or synced revision is checked against its hash, not scrubbed again, so a scrubber that learns a new pattern never rejects a library.

`playbooks.save` and `playbooks.remove` require both the displayed `expected_revision` and `expected_hash`. A stale editor gets a “changed since” error and keeps its edits; the apps then offer the latest version or saving theirs over it. Save activates a draft and appends a revision. Removal appends a tombstone and makes the skill unavailable. The preceding content and provenance remain in history. A removed skill is recreated under a new id.

An arriving blob merges into the library by revision id, keeps the revisions in revision order, preserves deletion records, and rejects a revision id whose content changes; a Device that has revisions the blob lacks sends the merged record back. Offline edits of one parent both survive; the largest revision number, then timestamp and revision id, select a deterministic current head, and a guarded edit creates the next revision. Deleting a bot or a group, here or in a roster from another Device, drops its skills from the library. First sync reads the kind with the other account records, and a Device that already consumed the relay log while ignoring it replays its slots once, as for attention and project context. The library is limited to 128 records, 128 revisions per record, and 2 MiB including history; a write exceeding a limit fails without discarding revisions.

## Discovery and lazy reads

When the turn's bot or group has saved skills, `turns.rs` adds their catalog through `playbook_tools.rs`: each skill's name, description, scope kind, and `playbook://<id>/SKILL.md` path, within 6,000 bytes, with an omitted count. Drafts and deleted records stay out, and a turn with no skills gets no section. Instructions, examples, and bundled files enter the model's context only through `read_playbook` after a match.

`list_playbooks { query?, sources? }` searches visible names and descriptions in a larger bounded result; `sources: true` additionally lists up to twenty completed user/current-bot text messages in the current chat with ids and short previews for capture. `read_playbook { path }` reads `SKILL.md`, generated with skill frontmatter and links to its bundled resources, or one exact resource path under the same `playbook://<id>/` directory. Each call resolves current scope and saved status again. Reading a project skill from a DM, another project, or after leaving the group fails.

## Workflow capture and standing instructions

A completed bot reply's context menu has **Save as Skill…**, and the user's own message **Save as Standing Instruction…**. The sheet lists the messages up to the clicked one, the last twelve that count: the user's and that bot's for a workflow, with the reply and the request before it picked, and the user's alone for corrections. A workflow needs a reply from the bot; a standing instruction needs two of the user's messages. In a group, **Used by** picks the group or the bot; group corrections go to the group's owner, or its first member. **Continue** asks for the draft and opens it for review.

`playbooks.draft { scope, chat_id, bot_id, kind, message_ids }` validates up to twenty selected messages, at most 24 KB of text. It excludes tool payloads, attachments, other bots, other chats, failed replies, and messages still streaming. Only the scrubbed selected text reaches the provider's small drafting model. The model has no tools; its strict content result is validated and saved as an inactive draft. The source is checked again after inference, and a timeout or invalid result leaves the library unchanged. Drafting usage counts in the source chat.

The bot's `propose_playbook` tool creates the same inactive draft from content it writes and validated evidence ids, and a chat notice says where to review it. Only the user's **Save** makes a draft available on later turns. Proposing or saving a standing instruction records provenance and revision history; it never adds an Auto-review rule or grants execution authority.

## App and API

The snapshot and `roster.changed` carry `playbooks`: every skill and draft without its body (`id`, `scope`, `name`, `description`, `status`, `revision`, `hash`, `updated_at`). Both apps' inspectors show a **Skills** section with the DM's bot's skills, or the group's own, drafts first and then the latest changed, and hide it while there are none. A row shows the name, two lines of the description, and Draft for a draft; a click fetches the record and opens its sheet, and the row's menu has Open, Export…, and Delete…. The section's + and Chat › New Skill… open an empty sheet. Past five rows the rest wait behind Show More, as in the Project and Tasks sections.

The skill sheet's subtitle says who can use the skill, and that a draft waits for a save. Name (typed words become a slug; a name the CLI refuses keeps Save off) and Description sit in the app's form grid over tabs: Instructions, Examples, References, Scripts, and, for a saved skill, History. A files tab is a list with + and − beside the selected file's text, a name renamed in place; a name the CLI would refuse goes back. History lists the revisions newest first in words (Created, Drafted from a chat, Saved, Edited, Deleted) with the date and Device, and shows the picked one's instructions. Save is the default button and Delete… sits apart on the left. Export writes the CLI's package through the system's save panel. On Windows and Linux the views are `sheet_playbooks.go`, with the records in `desktop/model/playbooks.go`.

The phone's Details has the same Skills section (`useSkills`), with an Add Skill menu: New Skill…, and From This Chat…, which stands in for the message menu, since a long press on a bubble selects its text there. A row slides in `app/chat-info/skill/[id]`: Name and Description with who can use it in the footer, Instructions and Examples as text fields, References and Scripts as rows that open a file's name and text (`skill-file`), History rows that show the instructions at each step, Export to the share sheet, Delete, and Save in the toolbar. From This Chat (`save-skill/[id]`) lists the chat's messages to tick: a bot's reply makes a workflow for that bot, two or more of the user's a standing instruction. The phone is never a Runner, so `playbooks.draft` goes to the bot's Runner as a sealed request, which drafts and syncs the draft back as its blob.

The local JSON methods are:

| Method | Parameters and result |
| --- | --- |
| `playbooks.get` | `{ scope, id }` → current `content`, `revision`, `hash`, `status`, metadata, provenance, and `revisions` |
| `playbooks.save` | `{ scope, id?, content, expected_revision, expected_hash, provenance? }` → current record; new records use revision `0` and empty hash |
| `playbooks.remove` | `{ scope, id, expected_revision, expected_hash }` → the deletion revision |
| `playbooks.draft` | Explicit source selection as above → inactive record for review |
| `playbooks.export` | `{ scope, id }` → portable allowlisted content and files |

The core supports lifecycle and export on every Device. Draft generation uses a Runner's local CLI, where the provider and drafting model are available. The public Rust `get` and `save` adapters use the same scopes and guards as the JSON API.

## Portable export

Export selects one saved playbook's current content. The result is version 1 of `lorca-playbook`: `{ format, version, content, files }`, where files contain generated `SKILL.md` and only that content's bundled references and scripts. The CLI scrubs the content again, preserving existing `«redacted credential»` and `«redacted N chars»` markers through repeated save/export/import cycles. Scope ids, account and Runner identities, chat messages, memory, history, provenance, provider configuration, plugin configuration, and credentials stay outside the export. A receiving workflow creates an independent scoped playbook from `content` using a new id and the usual reviewed save; portable content grants no permissions.
