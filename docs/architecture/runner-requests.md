# Requests to a Runner

What lives on a Runner (a bot's memory and working folder, its plugins and MCP servers, its secrets, a command it runs) is read and changed from any Device through questions the Device asks that Runner over the relay. `crates/cli/src/requests.rs` holds both sides.

## The envelopes

A question is a `kind=request` blob signed and sealed to one Runner (`{ id, verb, body }`, asked by its signer), answered with a `kind=response` the Runner signs and seals to the Device that asked (`{ request_id, body, error? }`); both are [signed envelopes](relay-trust.md#signed-envelopes). Each side deletes the blob it consumed, and the asker gives up after 20 s, or 75 s for the start of a plugin sign-in, whose discovery and registration the Runner cuts off at 45 s, or 150 s for `mcp.reconnect`, which answers once a server has connected or failed. A request is refused up front when the Runner is unknown or offline. The memory and working-folder verbs are answered only for a bot that runs there: a request that reached the wrong machine is refused, not forwarded.

## The verbs

- `feedback.*`: read, record, review, and decide workflow revisions on the assigned Runner.
- `memory.read` and `memory.write`: a bot's memory, shown and edited from a Device that is not its Runner.
- `workdir.files`, `workdir.changes`, and `workdir.diff`: a bot's [working folder](bots.md#working-folder), its files a folder at a time, what changed there, and a file's diff, read from any Device.
- `budgets.*` and `connector_limits.*`: a bot's limits and call limits, set from any Device.
- `plugins.install` / `plugins.rename` / `plugins.uninstall` / `plugins.variables` / `plugins.connect` / `plugins.sign_out` / `plugins.detail`: plugins on a Runner, managed from any Device; and `plugins.sign_in.finish` / `plugins.sign_in.cancel`, with which the Device that asked for a sign-in (its `redirect_uri` on `plugins.connect` or `permission.answer`) hands back where its browser landed (see Sign-in in [Codemode and Plugins](plugins.md#plugins)).
- The `mcp.*` methods but `mcp.parse`: a Runner's own MCP servers, managed from any Device.
- `permissions.catalog`: a bot's Access sheet lists its Runner's plugins and their tools.
- `permission.answer`: a permission card answered from any Device, and a secret request's values reach its Runner as ciphertext.
- `bash.stdin`, `bash.stop`, and `bash.background`: a command a bot runs, answered, stopped, or sent to the background from any Device.
- `coding.transcript` and `coding.stop`: a coding agent's transcript read, and the agent stopped, from any Device; `coding.agents` and `.set`: a Runner's coding agents ordered from any Device.
- `secrets.list`, `secrets.set`, and `secrets.delete`: a Runner's secrets, listed without their values, replaced, or deleted from any Device.
- `chats.send_now`: a message the bot's turn holds, read at once from any Device.
- The `events.*` methods but `events.forward`: a Runner's event subscriptions, managed from any Device.
- `service.status`: the Devices pane shows whether `lorca service` runs on a Runner.
- `update.install` and `update.auto`: a self-updating Runner's CLI installs its latest release, or turns its automatic updates on or off, from any Device.
