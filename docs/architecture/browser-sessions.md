# Browser sessions

`crates/cli/src/browser.rs` routes browser-session operations to the bot's assigned owned Runner. `browser/runner.rs` owns the persistent records, separate Playwright MCP processes, and exclusive input gates. The AppKit and native Go/MyGo desktop apps send these operations to their local CLI.

## Ownership and profiles

A session has a stable `browser-<uuid>` id, `bot_id`, `runner_id`, selected account and profile labels, `state`, `selected`, `revision`, and `created_at`. Account and profile are labels for the sign-in the user chooses inside that browser. Each new session gets a separate Lorca-generated Chromium profile directory. A session belongs to exactly one bot and the Runner recorded at creation. Every control and tool call checks the current bot assignment; a reassigned bot creates sessions on its new Runner. One session is selected per bot, and opening or returning a session selects it.

The Runner stores records in `browser/sessions.enc`, encrypted with the account DEK using `browser-sessions` as associated data, written through a private temporary file and rename. Restart recovers stable ids and labels with `stopped` control; it grants no bot input and opens no window. The Runner retains profiles locally, and sends session metadata to another paired Device only in its sealed response.

While open, Chromium uses the private `browser/profiles/<session id>` working directory. On Stop, the CLI closes the session's MCP process and archives that profile with XChaCha20-Poly1305 under the account key, binding the ciphertext to the session id. The encrypted archive replaces its checkpoint before the working copy is removed. Explicit open restores it. Archives contain regular files and directories, skip Chromium locks and regenerable caches, and cap retained data at 512 MiB. A failed checkpoint or abrupt process termination retains the private working copy so the sign-in remains recoverable; Stop can checkpoint it again. The CLI's ordinary shutdown closes sessions and checkpoints their profiles too. Forgetting the identity closes the session processes and removes their data with the account.

Profiles separate cookies, local storage, and browser history. The browser and tools run as the Runner's OS user. A dedicated Runner, or a Runner inside an OS isolation boundary, supplies stronger filesystem, process, and network separation through the bot's existing assignment.

## Visible open and takeover

The curated Browser plugin (`playwright`) supplies the tools and server package. Catalog discovery uses its dormant headless MCP connection. Explicit session open creates a separate headed stdio server, passing the session's `--user-data-dir` and a private output directory. Transport, CDP, extension, and profile overrides are refused for this managed backend. Opening selects the current tab through Playwright's core tab action and brings that page forward. The server uses Chrome by default; the Runner's `PLAYWRIGHT_MCP_EXECUTABLE_PATH` selects a compatible Chromium executable when configured. A user open grants human control; a reviewed bot open grants bot control. Returning control preserves the same server, tabs, sign-in, page, and model/task context.

Control moves between `stopped`, `bot`, `taking_over`, and `human`. Take Over changes `bot` to `taking_over` before waiting for the input gate. Each active browser call holds that gate through its MCP response; the CLI grants `human` only once active input drains. New Browser calls and subsequent calls at the turn's tool boundary wait for explicit Return to Bot. Waiting calls retain their futures and codemode state, and chat Stop cancels them. A bot cannot create or open another session to override its human-controlled browser.

Return to Bot requires the current revision and `human` state, checks the current ownership again after taking the gate, and wakes waiting calls. A stale request cannot return a newer takeover to the bot. Browser tool execution checks ownership, selection, and control again after waiting, so a permission approval cannot grant input after takeover.

Stop revokes input immediately, cancels an in-progress open, and closes only that session's server/browser. With no active input it closes Chromium gracefully to flush storage; during active input it closes the process. A cancelled or timed-out browser tool likewise closes its session's process before releasing the gate, because MCP cancellation alone acknowledges no completion of input. Stop preserves the profile and chat context. The sheet keeps Stop enabled while Open or Take Over waits.

Bot `browser_session { action: list | create | open, session_id?, account?, profile? }` uses the same ownership checks. Creating and opening go through [Auto-review](tools.md) at the loop's `before_tool_call` boundary, as plugin actions do; a held operation in an unattended routine is refused. The Browser plugin's calls keep their existing live-description/read-only review rules. The bot's codemode catalog binds actual Browser calls to its selected session, rather than the Runner-wide catalog connection.

## Device capabilities

`browser.sessions`, `browser.create`, `browser.open`, `browser.takeover`, `browser.resume`, `browser.stop`, and `browser.screenshot` take `bot_id`. Session operations also take `session_id`; resume requires `revision`, and screenshots require `chat_id`. Create takes the account and profile labels. The screenshot chat must contain the session's bot. Methods return a session or evidence message id and explicit capabilities. Sessions returns the list and those capabilities.

| Capability | Local assigned Runner | Another paired Device |
| --- | --- | --- |
| List/create profiles | Local CLI | Sealed request and response |
| Open visible browser | Runner screen | Open on the assigned Runner |
| Pause/takeover | Exclusive input gate; user interacts on Runner | Pause the Runner's bot input; interaction occurs on Runner |
| Return to Bot / Stop | Local CLI | Sealed request and response |
| Verification screenshot | Encrypted chat attachment | Request capture on Runner; encrypted attachment |
| Live remote view/input | `remote_live_view: false`, `remote_input: false` | Same explicit capabilities |
| Native application input | `native_input: false` | Same explicit capability |

Paired control uses existing `request` and `response` sealed envelopes, checks that the Runner is known and online, and allows 150 seconds for process startup or active-input drain. A timeout is an unknown operation outcome: refresh the session before repeating a transfer of control. These capabilities leave the bot's Runner assignment intact.

## Screenshot evidence

Attach Screenshot, and a successful bot `browser_take_screenshot`, create a new immutable `Body::Text` chat message with a PNG attachment and the session id. The outcome reads unverified: a screenshot captures the page and carries no automatic claim that a task passed. The CLI copies bytes into the existing private attachment store, queues the account-encrypted `file` blob before the encrypted chat message, and associates both with that chat. Paired Devices retrieve the evidence through `files.path`. Publishing introduces no task records or external uploads.

The AppKit inspector's Browser Sessions sheet shows owner and Runner, account/profile choices, stable session id, control state, Open Browser, Take Over or Pause on Runner, Return to Bot, Stop Browser, and Attach Screenshot. A paired Mac states that sign-in and input happen on the assigned Runner. It refreshes while visible and does not advertise live viewing or native input.

The Windows/Linux inspector opens the same controls in `desktop/sheet_browser.go`. `desktop/model/browser.go` reads the CLI's session, revision, and capability fields and sends the bound bot/chat and selected profile. Capability flags determine which actions are enabled; a missing flag grants no input. Account/profile drafts and the selected id persist across frames and refreshes with stable control keys. Replies and refresh timers use the store's ordered main-thread queue. Dismissal stops polling and ignores late replies; a later Stop invalidates older takeover/open callbacks, completed actions discard older polls, and a changed Runner assignment disables the sheet's actions. The in-process demo retains synthetic profiles in memory, opens no browser, and produces no screenshot evidence.
