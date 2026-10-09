# Bot permissions

A bot's Access belongs to the user. It limits which plugins on the bot's Runner the bot may use, how far (read, draft, write), down to single tools, and whether the bot reads or changes files and runs shell commands there. The CLI checks it before [Auto-review](tools.md) and again when a call runs; an account rule, Allow once, or Always allow cannot widen it. A bot's description tells it what to do; its Access decides what its tools may do.

## Profile and wire shape

`Bot.permissions` lives in the encrypted `roster` blob with the rest of the profile. Provider credentials stay in the account's encrypted `credentials` blob, and plugin tokens and configuration stay on their Runner; the policy holds no secrets.

```json
{
  "connections": {
    "mail-work": {
      "capabilities": ["read", "draft"],
      "tools": ["list_messages", "create_draft"]
    }
  },
  "filesystem": "read",
  "shell": false
}
```

`connections` is keyed by the installed plugin's id on the bot's Runner, so two accounts of one service, installed as two plugins, get separate grants. Without `connections` the bot may use every plugin on the Runner, one installed later included; with it, only the plugins it lists, each with a non-empty `capabilities` set. A grant's `tools` holds original MCP tool names; without it every tool of that plugin is allowed, one the server adds later included. Names match exactly. `filesystem` is `none`, `read`, or `write` (default `write`), and `shell` is a boolean (default true). A bot with no policy has full access. `crates/cli/src/permissions.rs` holds the types and the checks.

The three capabilities are separate grants: `write` does not take in `read`. The apps offer them as levels that each take in the ones below (Read only; Read and draft; Read and write; No access), and write those sets. A tool needs `read` when its live server marks it `readOnlyHint`, its manifest lists it in `tools.readonly`, or it is one of a server's resource readers; `draft` when the manifest lists it in `tools.draft` (a draft still crosses Auto-review); `write` otherwise. What `catalog.json` saved never decides a capability at execution, since a bot can write that file: the check connects the server, through the pool, and reads the live mark.

`bots.create` and `bots.update { id, permissions }` take the user's policy. An update leaves it alone when the field is absent and refuses `null` and malformed fields. A user gives full access back by saving the default policy. A roster from a Device that predates policies keeps the policy the Device already had (`keep_policies`) and uploads it again. `create_bot` and `edit_bot` take no policy and refuse one; a teammate a bot creates gets its creator's current policy. A handoff (`message_bot`) asks another bot to work with that bot's own Access, and a group gives each member a turn under its own.

## Execution boundary

- `check_tool(app, bot, name)` reads the bot as stored now and checks `shell` for `bash`, `bash_input`, and `bash_output`, and `filesystem` for the coding tools: `read`, `grep`, `find`, and `ls` need `read`; `write` and `edit` need `write`.
- `check_connection(app, bot, plugin_id, tool, capability)` checks the plugin and the exact tool, then, given one, the capability. With no capability it runs before a server starts to classify the tool.
- A bot deleted, or moved to another Runner, during a turn makes no more calls; that refusal asks nobody.

`TurnHooks::before_tool_call` checks a local tool before Auto-review's fast paths, saved rules, or model judgment; `mcp::review_call` does the same for a plugin call. Local tools are wrapped (`permissions::guarded`) to check again when they start, and a plugin tool bound to the bot (`mcp::bot_catalog`) checks again after its server connects and right before the request, so access taken away mid-turn stops the next call. A call already running finishes or is stopped through the usual cancellation. Codemode's nested calls go through the same hooks and wrappers one by one: allowing a script allows none of its calls. A refused call ends the script. A routine's check reads only, and only what the bot's Access allows (`mcp::authorize_catalog_tool`).

A bot's turn leaves out the plugins its Access does not allow: the system prompt does not name them, and its catalog neither lists nor connects them, so a search or a call by name starts nothing. `connect_plugin` refuses such a plugin too.

## Requests to the user

A refusal returns the reason to the model, which tells the user, and puts a request in the chat: a `permission` message with `tool: access`, the plugin's id and name (or `computer` for files and shell), and a summary the apps word ("GitHub · create_issue", or Shell commands, Changing files, Reading files in the app's language). One request waits per missing grant. It never waits in the tool approval queue and never allows the call: `permission.answer` refuses Allow and Always allow for it, and Dismiss records it as `dismissed`. Saving the bot's Access dismisses its open requests (`dismiss_requests`) and resumes nothing; the bot's next call is checked against the new policy. A request sends no push of its own, and the apps raise no notification for it: the bot's reply does.

On the Mac and on Windows and Linux, the bot's DM Profile card has an Access row ("Full access" or "Limited"; `DisclosureRow` on the Mac, `disclosureRow` in `desktop/`) that opens the Access sheet (`Sheets/BotAccessViewController.swift`, `desktop/sheet_access.go`) on a click anywhere, and a request offers Edit Access…, which opens the same sheet, and Dismiss. The phone shows the request with Dismiss. The sheet lists the Runner's plugins, each with its level in a pop-up and, once the plugin has connected, a disclosure for its tools: a checkbox per tool, with what it does (Reads, Drafts, Changes), off and dimmed while the level does not reach it. Under the Runner's name are Files (Read and write, Read only, No access) and a Shell commands switch, with the note that shell commands run as the user and can reach anything the user can. Save writes the policy through `bots.update`; every plugin at Read and write with all its tools saves no `connections`. `bots.permissions` gets the plugins and their saved tools from the Runner (`permissions.catalog`, through the sealed request path when it is another Device); while it answers, or when it cannot, the sheet lists the plugins the Runner advertises. The inspector's Plugins section says No access for a plugin the bot may not use.

## Shell, filesystem, and isolation

`workdir` is where relative paths start. Shell and coding tools run as the Runner's user, with that user's files, environment, processes, credentials, and network. Files read reaches any path that user can read; files write, any path that user can write. Shell access runs any command, whatever the file setting. The policy gates the CLI's own tools and is no OS sandbox.

Shell commands, file access, the user's own stdio servers, and tools that run arbitrary code can reach the Runner's account files, plugin secrets, and its CLI outside these checks, so plugin limits are as strong as a bot's shell and file access: with both off, a bot is kept to its plugins. Stronger separation is an OS-isolated process with its own credentials and files, or a dedicated Runner that has only the plugins and data that bot may reach, picked through the bot's Runner assignment. Process groups serve cancellation and isolate nothing.
