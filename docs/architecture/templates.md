# Bot templates

A bot template is a file (`.lorca-template`, UTF-8 JSON) holding the parts of a bot the user picked: its profile, skills, memories, routines, and the plugins it uses. Any bot exports to one, and a new bot is made from one on any Runner of any account. `crates/cli/src/templates/` reads and writes the files and checks them; the apps ask their local CLI. Exporting writes a file where the user saves it and publishes nothing; the file is plaintext, private only as far as the user keeps it.

## What a file holds

The root is `format: "lorca.bot-template"`, `version: 1`, an optional `profile`, and the arrays `skills`, `memories`, `routines`, and `requirements` (omitted when empty). Nothing else has a field:

- **Profile**: name, description (the bot's instructions), SF Symbol, and accent. No avatar, provider, model, Runner, or working directory.
- **Skills**: name, description, instructions, examples, and inline `references/` and `scripts/` texts, as the playbook API's content.
- **Memories**: lines of the bot's `workspaces/<bot id>/MEMORY.md` and whole `memory/*.md` topic files. Daily logs, attachments, and other workspace files are never offered. Reads are bounded, refuse symbolic links, and ignore a working directory's memory.
- **Routines**: name, schedule, prompt, the check script, and `timezone` and `missed_run_policy` where the routine API has them. No run history, health, or next run.
- **Requirements**: a `service_id` per plugin. Never the plugin's variables, tokens, or account.

A file is at most 1 MiB. Parsing refuses unknown fields at any level, another version, invalid schedules, duplicate names, oversized text, NUL characters, and resource paths outside `references/` and `scripts/`. Skills keep to the playbook limits, memories to the 256 KiB memory file, routines to a bot's routine and check limits. This format is separate from the [marketplace index](marketplace.md#the-index).

## Export

`templates.contents { bot_id }` lists what the bot has, each piece as the file would hold it: `profile`, `skills`, `memories`, and `routines` as `{ id, content, flags }`, and `requirements` as `{ service_id, name }`, one per plugin on the bot's Runner. Every text is redacted the way the export redacts it: recognizable credentials by pattern, and the exact values of the account's provider credentials and the Runner's plugin secrets, which are read only to be removed. `flags` name what a reader should look at before sharing: `email`, `phone`, `path`, `link`, and `credential` (a key that was removed). Skills are listed when the CLI has the playbook store (`playbooks.list` / `playbooks.export`); without it there are none to offer.

`templates.export.preview { bot_id, selection }` builds the file's contents from `selection` (`profile`, and the ids in `skill_ids`, `memory_ids`, `routine_ids`, `requirement_ids`) and answers `{ template, digest }`. A plugin's tool names in the text (`github__search`) are rewritten to the service's, and a source account's instance id to `{{connection:<service>}}`. Content that uses a plugin whose requirement is not picked is refused, and so is content using two accounts of one service. `templates.export { …, path, expected_digest, reviewed: true, overwrite? }` builds it again, refuses contents that changed since the preview, and writes the file atomically with mode 0600 on Unix; an existing file needs `overwrite`, and a path inside Lorca's data folder is refused.

## Import

`templates.import.preview { path, runner_id, mappings? }` reads the file and answers its `template` (each routine with its `schedule_text`), `digest`, `issues`, `can_import`, and per requirement `{ service_id, name, candidates, selected }`: the Runner's own installed plugins for that service, and the one picked, which is the `mappings` choice or else the Runner's only ready one. A requirement without a ready pick blocks the import without an issue of its own; the apps say on its row what it needs. `issues` are the file's problems: an unreadable or unsupported file, text that looks like a credential (the import refuses it rather than change what was reviewed), routine fields this CLI lacks, and skills without the playbook store. Import installs no plugin and signs in to none.

`templates.import { path, runner_id, name, provider, mappings, expected_digest, reviewed: true }` checks the file and the Runner's connections again and makes a new bot and its DM: the template's profile under the given name, the provider picked, the default workspace, the picked memories as its `MEMORY.md`, its skills saved through the guarded playbook API in the new bot's scope (`template_import` provenance), and every routine created with `enabled: false`. Tool names and `{{connection:…}}` references resolve to the recipient's picks. No greeting, turn, or routine check runs. A failure removes the bot, its routines, its workspace, and the skills already saved. The reply is `{ bot, chat_id }`, and the bot shares nothing with the one it came from.

## Bots on another Runner

Memory and plugin secrets live on a bot's Runner, so the CLI asks it through sealed [requests](protocols.md#cli--relay): `templates.memories` for the memory candidates (redacted there) and `templates.export.preview` for the preview of a remote bot. An import for another Runner sends the file's text and the recipient's choices in a `templates.import` request; that Runner checks them again and creates the bot, which reaches every Device through the encrypted roster.

## In the apps

The macOS app and the Windows and Linux app are the same here. **File › Export as Template…**, a DM's context menu, and the command palette export the bot of the DM that is showing. The sheet, titled with the bot's name, lists Profile, Skills, Routines, Plugins, and Memories as cards of rows to check, each row showing what the file will hold and its flags at the end (an email address in orange, a removed key in gray); the profile starts checked, and Memories has Select All once it has three. **Export…** asks for the preview, so the CLI's objections show under the cards before the Save dialog opens, then saves.

**File › New Bot from Template…** opens the system's Open dialog, then the New Bot form: Name, Runner (this computer first), Provider, and a line per plugin with a pop-up of the Runner's connections for it, or that the Runner has none. Below it the template's contents show as the same cards, read-only. The sheet previews again when the Runner, a connection, or the Runner's plugins change; one line says what is missing, else that routines start paused. **Create Bot** opens the new bot's DM.
