# Bot permissions

A bot's Access policy belongs to the user. It limits the connection instances, tools, and capabilities the bot can use on its assigned Runner. The CLI enforces it before [Auto-review](tools.md) and again at execution; an account rule, Allow once, or Always allow cannot widen it. A bot's description supplies instructions, while this policy controls tool execution.

## Profile and wire shape

`Bot.permissions` lives in the existing encrypted `roster` blob and the Device's private local state. Provider credentials remain the account's encrypted `credentials` blob; plugin tokens and configuration stay on their Runner. The policy contains no secrets.

```json
{
  "connections": {
    "mail-work": {
      "capabilities": ["read", "draft"],
      "tools": ["list_messages", "create_draft"]
    }
  },
  "tools": ["codemode", "read", "recall"],
  "filesystem": "read",
  "shell": false
}
```

`connections` is an optional map keyed by the installed plugin's stable instance ID on the bot's Runner. Two accounts of a service have different IDs and different grants. Its optional `tools` set holds original MCP tool names, before their JavaScript identifier conversion. Local CLI tools have their own optional `tools` set. An omitted allowlist permits every name in that scope; an empty one permits none. Exact names match: a tool added to a server later does not acquire another tool's grant.

The Access catalog also lists `stage_review` so explicit policies can select the review-staging tool on Runners that provide it. Permission to stage a review grants no shell or connection execution capability; an approved action still needs those grants at execution.

The three connection capabilities are independent: `read`, `draft`, and `write`. Selecting write does not select read or draft. An empty capability set denies the connection. `filesystem` is `none`, `read`, or `write` (default `write`); `shell` is boolean (default true). A missing policy preserves a bot's existing full access. Invalid fields, capabilities, and allowlist values are refused by the user's API. A user restores full access by saving an explicit default policy; a roster from a Device that omits the policy retains the known policy and uploads it again.

`bots.create` and `bots.update { id, permissions }` accept the user's policy. Updates leave it alone when the field is absent and reject `null`. `edit_bot` and `create_bot` expose no policy field and reject one even when called directly. A bot-created teammate inherits its creator's current policy, including restrictions the user changed during the turn. Handoff is its own local `message_bot` grant: disable it when a specialist must not ask other bots to work with their own access. A group offers each member a turn with that member's own policy.

## Execution boundary

`crates/cli/src/permissions.rs` owns the policy types and reusable checks:

- `check_tool(app, bot, name)` reads the latest stored profile, checks the local tool allowlist, and checks shell or filesystem grants for the coding tools.
- `check_connection(app, bot, connection_id, original_tool_name, capability)` checks instance, exact tool, and capability grants. `check_connection_tool` checks instance and tool selection before a server starts for classification.
- `policy_fingerprint(app, bot)` returns lowercase SHA-256 of the versioned effective policy, bot ID, and Runner assignment. Unrelated name, look, and model edits leave it unchanged. The fingerprint binds an approval precondition and does not replace fresh authorization of the exact call.
- `plugins::mcp::authorize_tool` authorizes a local tool or an exact codemode MCP name with the live/trusted classification. `bot_catalog` binds MCP tools to a bot and chat for the final execution check.

A deleted bot has no access, and a turn bound to a previous Runner stops making calls after reassignment. Every check reads the current policy rather than the turn's snapshot, so revocation during a turn applies to subsequent calls. `TurnHooks` checks before Auto-review's fast paths, saved exact rules, or model judgment. Local tool wrappers recheck when execution begins. MCP tools recheck after a server connects and immediately before the request. Calls already executing complete or are stopped through the existing cancellation controls.

Codemode's nested calls cross the same loop hooks and execution guards as top-level calls. Each child call is checked separately, including shell and file calls; allowing `codemode` does not authorize its children. A denied call at the loop boundary ends the script and prevents its waiting calls from starting. Routine checks use the same bot policy in addition to their read-only restriction, and memory housekeeping uses guarded memory tools.

An MCP tool requires read when its live server supplies `readOnlyHint`, its manifest lists it in `tools.readonly`, or it is a protocol resource reader. Draft requires the manifest's `tools.draft` declaration; it still goes through Auto-review. Every other effectful tool requires write. Cached annotations and names such as `get_*` or `draft_*` never establish capability at execution. The profile catalog uses saved labels to display tools, with this distinction explained in the editor. Connection discovery cannot start a bot-excluded instance. Resource readers share the connection's tool and read grants.

## Requests to the user

A refusal names the denied tool, connection or capability and explains the restriction. The Runner records an encrypted permission message with `tool: access`, pointing to the bot's profile. Repeated refusals for the same missing grant reuse a pending request among the recent messages. This request does not wait in the tool approval queue and has no action approval: it opens the profile editor or is dismissed. `permission.answer` refuses Allow and Always allow for it; Dismiss records a denial. Saving the bot's access dismisses its recent outstanding requests and never resumes a refused call. A later call checks the saved policy again.

On macOS the Profile card has an Access summary and Edit button. The AppKit sheet offers all connections or an explicit selection of the Runner's advertised connection instances, independent Read/Draft/Write checkboxes, all tools or exact tool selections for each connection, a local tool checklist, filesystem access, and shell access. `bots.permissions` asks the assigned Runner for `permissions.catalog` through the existing sealed request path; the local CLI supplies the profile and local names. The editor can inspect saved grants when that Runner is offline. Edits use `bots.update` through the local CLI and sync with the roster. An access request offers Edit Access and Dismiss, and never offers Allow once or Always allow.

The Windows/Linux MyGo app provides the same Profile Access summary and native editor (`desktop/sheet_access.go`). Its `desktop/model/permissions.go` policy types preserve absent versus empty allowlists, and its persistent draft keeps edited fields and exact instance/tool names across native build passes, language changes, and catalog reloads. Stable keys identify each instance, capability, and tool. Replies use the store’s ordered main-thread queue; a dismissed sheet or superseded catalog request ignores late replies. Save sends the same `bots.update` policy through the local CLI, changes only the returned policy after confirmation, and keeps the draft visible on errors or a concurrent policy change. A stale save reply does not overwrite newer roster policy. Refused-access cards open this editor or send `deny` to dismiss; the desktop store sends no Allow/Always action for an access request.

## Shell, filesystem, and isolation

`workdir` is the context for relative paths. The shell and coding tools run as the Runner's user, with that user's filesystem, environment, processes, credentials, and network access. Filesystem read can read any path that user can read; filesystem write can modify any path that user can write. Shell access allows arbitrary commands even when file tools are restricted. The policy gates the CLI's tools and supplies no OS sandbox for code the user authorizes.

Shell commands, unrestricted file writes, user-added stdio servers, and tools that execute arbitrary code can reach the Runner's account files and local CLI outside these gates. Connection restrictions therefore provide tool-level separation for specialists with those paths disabled. Stronger separation uses an OS-isolated process with restricted credentials and filesystem access, or a dedicated Runner configured with only the integrations and data that bot may reach. The user selects that Runner through the bot's existing assignment. Ordinary process groups support cancellation; they do not isolate credentials or filesystem access.
