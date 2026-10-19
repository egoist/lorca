# Requests between Devices

A Device asks a Runner for work on that Runner through the relay: a question goes up as a `kind=request` blob, [signed](relay-trust.md#signed-envelopes) and sealed to one Runner (`crates/cli/src/requests.rs`: `{ id, verb, body }`, asked by its signer), and the Runner answers with a `kind=response` it signs and seals to the Device that asked (`{ request_id, body, error? }`). Each side deletes the blob it consumed. The asker gives up after 20 s, or 75 s for the start of a plugin sign-in, whose discovery and registration the Runner cuts off at 45 s, or 150 s for `mcp.reconnect`, which answers once a server has connected or failed. A request is refused up front when the Runner is unknown or offline.

## Verbs

- `feedback.*` read, record, review, and decide workflow revisions on the assigned Runner ([Workflow feedback](feedback.md)).
- `memory.read` and `memory.write`, so a bot's memory can be shown and edited from a Device that is not its Runner.
- `budgets.*` and `connector_limits.*`, so a bot's limits and call limits are set from any Device.
- `plugins.install` / `plugins.rename` / `plugins.uninstall` / `plugins.variables` / `plugins.connect` / `plugins.sign_out` / `plugins.detail`, so plugins on a Runner are managed from any Device, and `plugins.sign_in.finish` / `plugins.sign_in.cancel`, with which the Device that asked for a sign-in (its `redirect_uri` on `plugins.connect` or `permission.answer`) hands back where its browser landed (see Sign-in in [Codemode and Plugins](plugins.md#plugins)).
- The `mcp.*` methods but `mcp.parse`, so a Runner's own MCP servers are managed from any Device.
- `permissions.catalog`, so a bot's Access sheet lists its Runner's plugins and their tools; `permission.answer`, so a permission card is answered from any Device and a secret request's values reach its Runner as ciphertext.
- `bash.stdin`, `bash.stop`, and `bash.background`, so a command a bot runs is answered, stopped, or sent to the background from any Device.
- `coding.transcript`, `coding.send`, and `coding.stop`, so a coding agent is read, written to, and stopped from any Device; `coding.agents` and `.set`, so a Runner's coding agents are ordered from any Device.
- `secrets.list`, `secrets.set`, and `secrets.delete`, so a Runner's secrets are listed without their values, replaced, or deleted from any Device.
- `chats.send_now`, so a message the bot's turn holds is read at once from any Device.
- The `events.*` methods but `events.forward`, so a Runner's event subscriptions are managed from any Device.
- `service.status`, so the Devices pane shows whether `lorca service` runs on a Runner.
- `update.install` and `update.auto`, so a self-updating Runner's CLI installs its latest release, or turns its automatic updates on or off, from any Device ([Updates and the service](runtime.md#updates-and-the-service)).
- `app.update.install`, so the desktop app on a Runner installs its latest version and relaunches into it once no bot is at work there ([Updates from another Device](desktop-app.md#updates-from-another-device)).
