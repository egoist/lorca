# Integrations and named accounts

Slack, Gmail, Google Calendar, and Google Drive are [marketplace](marketplace.md) plugins backed by the services' streamable HTTP MCP servers. Settings › Plugins, the marketplace, and a bot's inspector manage them through the local CLI. The selected Runner holds each account's installation, setup, and authorization.

## Services and authorization

| Marketplace ID | MCP endpoint | Authorization |
| --- | --- | --- |
| `slack` | `https://mcp.slack.com/mcp` | Slack native user OAuth with PKCE; `SLACK_CLIENT_ID`, and `SLACK_CLIENT_SECRET` when the registered client requires it; loopback callback port 3118 |
| `gmail` | `https://gmailmcp.googleapis.com/mcp/v1` | Google Desktop OAuth with PKCE; `GOOGLE_CLIENT_ID`, and `GOOGLE_CLIENT_SECRET` when the registered client requires it |
| `google-calendar` | `https://calendarmcp.googleapis.com/mcp/v1` | The same Google Desktop OAuth flow, with Calendar scopes |
| `google-drive` | `https://drivemcp.googleapis.com/mcp/v1` | The same Google Desktop OAuth flow, with Drive scopes |

Google's MCP endpoints require a project enrolled in the Google Workspace developer preview with the service API and its MCP API enabled. The setup fields accept a registered Desktop OAuth client's settings from that project. Google asks for offline access and opens its account picker at each sign-in (`access_type=offline`, `prompt=consent select_account`). Gmail requests read, compose, and modify scopes; Calendar requests calendar-list read, events, and free/busy scopes; Drive requests read and app-created-file scopes. A Slack client has PKCE enabled, its user scopes, its registered `http://localhost:3118/callback` redirect, and workspace approval for MCP. The service's setup and eligibility are documented by [Slack](https://docs.slack.dev/ai/slack-mcp-server/connect-to-harnesses/), [Gmail](https://developers.google.com/workspace/gmail/api/guides/configure-mcp-server), [Calendar](https://developers.google.com/workspace/calendar/api/guides/configure-mcp-server), and [Drive](https://developers.google.com/workspace/drive/api/guides/configure-mcp-server).

The native service flow (`crates/cli/src/plugins/oauth.rs`) uses the manifest's `authorization_endpoint` and `token_endpoint`. The Runner generates the random state, PKCE verifier, and S256 challenge, and exchanges the callback code for tokens. `authorization_params` supplies service consent options; a manifest cannot override state, the redirect, client id, response type, scopes, or PKCE parameters. The callback must have the expected origin, path, and a single matching state. Token exchanges and refreshes send the service's OAuth fields. The service uses those tokens as MCP bearers.

Sign-in on a paired Device uses the existing encrypted [request flow](protocols.md#cli--relay): that Device reads the account's public callback settings from `plugins.detail`, binds its loopback (Slack's registered port included), and sends its `redirect_uri` in `plugins.connect` or `permission.answer`. The Runner returns only the consent page and sign-in id, holds the verifier, and accepts the callback through `plugins.sign_in.finish`. The code and response travel sealed to the assigned Runner. Closing the page sends `plugins.sign_in.cancel`; a changed configuration, sign-out, or removal invalidates a pending authorization's generation. A late callback cannot restore a sign-in that the user has removed.

Tokens, refresh tokens, client secrets, and pending PKCE state belong to the Runner. Durable secrets use the plugin store's private `secrets.json`; they do not enter the account's provider `credentials` blob. The machine advertisement and plugin detail carry descriptive status, account names, and which setup fields are set. Secret setup fields are write-only.

## Stable account instances

A manifest with `named_accounts: true` installs an instance through `plugins::accounts`, reusing `Installed`, the MCP pool, and the plugin store. Its id is `<service-id>-<32 lowercase UUID hex characters>`. `Installed.manifest.id` and `PluginStatus.id` are that instance id; `service_id` remembers the marketplace entry and `account_name` is its label. These optional fields preserve existing plugin files and older machine advertisements. A label has 1–80 characters without controls and is unique, ignoring case, within its service on the Runner. Renaming, reauthorization, restart, and a marketplace manifest refresh keep the instance id. Refresh matches by service id, then restores the instance id before updating the manifest.

- `plugins.install { runner_id, plugin_id: service_id, account_name? }` creates another instance and returns `{ status }`. An omitted name becomes the next unused `Account N`.
- `plugins.rename { runner_id, plugin_id: instance_id, account_name }` returns `{ status }` with the new label.
- `plugins.detail`, `plugins.set_variables`, `plugins.connect`, `plugins.sign_out`, and `plugins.uninstall` address the instance id. Sign-out and removal affect that instance's tokens and tools.

The account name, service id, and state are advertised inside the Runner's encrypted machine blob, so paired Devices show the same accounts. Templates name the service requirement; setup recognizes instances by `service_id`. The AppKit and native Go/MyGo marketplaces open an Accounts sheet for a named service. Its rows show each account and status with Manage; Add Account asks for a name and opens that instance's plugin sheet. The plugin sheet edits the name, setup, and sign-in and opens Manage Accounts. Settings and the inspector list individual instances by service and account name.

The native Windows/Linux model (`desktop/model/integrations.go`) maps optional `service_id`, `account_name`, and `named_accounts` fields and keeps the returned instance status in the Runner's existing plugin list on the ordered main-thread reply queue. The account picker (`desktop/sheet_integration_accounts.go`) keys each row by instance id and opens that exact instance. Its Add Account prompt and the plugin sheet's name field keep their drafts in sheet state. A roster refresh preserves an edited name, and a closed sheet ignores later UI replies. Saving a name uses `plugins.rename`; edited setup variables use `plugins.set_variables`, so a name-only edit leaves authorization settings alone. The sign-in, sign-out, setup, and recovery controls use the same CLI methods as the AppKit app.

## Tool selection, review, and sources

Each instance is its own codemode namespace, such as `gmail_<uuid>__get_thread`. The catalog names its service, account label, and stable id. The bot selects that namespace explicitly and asks when the intended account is unclear. An authorization failure remains a failure for that account. `connect_plugin` accepts the instance id or its complete display name. The bot's install tool reports existing account ids and uses a newly installed instance's id for its sign-in card.

Auto-review uses the existing live plugin boundary before the MCP call. Permission cards name the service and account, and exact Always allow rules use `<instance-id>/<original-tool-name>`. `tools.readonly` gives trusted read hints; `tools.draft` identifies unsent draft operations for bot policies. Curated hints list known operations explicitly. Draft creation and other effectful operations pass through Auto-review; a draft hint does not grant automatic approval. MCP tools added by a service without a trusted read/draft declaration retain the normal effectful review boundary.

The script receives the whole MCP `CallToolResult`, including `structuredContent`, text, embedded resources, and `resource_link` blocks. Service source URLs, including message permalinks, event `htmlLink`s, and document `webViewLink`s, remain in that result. Each integration's skill tells bots to retain clickable sources in replies and summaries and to keep them associated with the account that returned them.

## States and recovery

Each account advertises `needs_setup`, `needs_auth`, `connecting`, `ready`, `insufficient_access`, or `error`. A ready named account reads Connected. A missing client setting reads Needs setup, while a missing or expired authorization reads Sign in. A refused scope is persisted under the account's server in the private store, reads insufficient access, and is included in the next consent request. Successful authorization clears that state. Network failures read Error, with another sign-in available in the account sheet.

Native access tokens are refreshed before their pooled bearer expires. Refresh calls run under the existing per-instance/server connection gate, retain an unrotated refresh token when omitted by the server, and persist a rotated pair before a new MCP connection uses it. An expired or revoked refresh token forgets that instance's authorization and requests another sign-in. Token endpoint errors use fixed recovery messages so a response that echoes a code or token cannot put it in a status. MCP tool calls are never automatically replayed as part of recovery.
