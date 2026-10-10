# Browser sessions

A bot's browser profiles: each keeps its own sign-ins on the bot's Runner, opens in a window there, and lets the user take the browser from the bot and hand it back. `crates/cli/src/browser.rs` answers the `browser.*` methods and forwards them to the bot's Runner; `browser/runner.rs` keeps the records, the open browsers, and their input gates.

## Profiles

A profile (`Session` in the code) has a stable `browser-<uuid>` id, `bot_id`, `runner_id`, a `name`, `state` (`stopped`, `bot`, `taking_over`, `human`), `selected`, `revision`, and `created_at`. It belongs to one bot on the Runner it was made on. Every method and every Browser call checks the bot's current assignment, so a bot moved to another Runner starts over with that Runner's profiles.

The Runner keeps the records in `browser/sessions.enc`, encrypted with the account DEK (`browser-sessions` as associated data), written to a private file and renamed into place. Each profile's Chromium folder is `browser/profiles/<id>` (0700): its cookies, storage, and history stay there between opens, on that Runner only, as the plugins' secrets do. A restart leaves every profile closed and opens no window. Deleting a profile closes its browser and removes its folder; forgetting the identity removes `browser/`.

A bot's first profile is its `selected` one, and opening another selects that. The bot's Browser calls go to its selected profile while that is open with the bot in control (`bot`). With none open they go to the plugin's shared headless browser, as for a bot without profiles, so Browser works on a Runner without a screen.

Profiles keep sites' sign-ins apart; the browser and the bot's tools still run as the Runner's user. A dedicated Runner, or a Runner inside an OS isolation boundary, is the stronger separation, through the bot's assignment.

## Opening

The curated Browser plugin (`playwright`) supplies the server. Opening a profile starts a headed Playwright MCP process of its own (stdio, `--user-data-dir browser/profiles/<id>`, `--output-dir browser/output/<id>`, PNG image answers), then brings its current tab forward; opening an open profile only brings it forward. A Browser plugin whose arguments or environment set their own profile, config, CDP endpoint, extension, or port can't open profiles, and those `PLAYWRIGHT_MCP_*` variables are taken out of the login shell's environment for these processes. Chrome is the default browser; the Runner's `PLAYWRIGHT_MCP_EXECUTABLE_PATH` picks another Chromium.

The apps' Open Browser gives the user control (`human`). The bot opens one with `browser_session { action: "open" }`, which passes [Auto-review](tools.md) as a plugin action does (refused in a routine nobody watches) and gives the bot control (`bot`). Profiles are part of the Browser plugin for a bot's [Access](bot-permissions.md): a bot whose Access leaves Browser out has no `browser_session`, and every call of it checks the grant first (`permissions::check_plugin`, any grant to the plugin, whichever of its tools the grant lists). Closing (`browser.stop`) revokes the bot's input at once and cancels an open still starting; with no call in flight it closes Chromium first so it writes out its cookies, then ends the process. Removing the Browser plugin closes every profile's browser.

## Taking over

Every Browser call in a profile holds that profile's input gate until its server answers. Take Over marks the profile `taking_over` at once, so new calls wait, and `human` once the call in flight has answered; on the Runner the window comes forward. Waiting calls keep their turn and codemode state, and the chat's Stop ends them. `browser_session` waits during a takeover too, and the bot can't open a profile over the user. The bot's other tools run as usual.

Return to Bot sends the revision the app last saw: a profile that changed since (another Device took it over again, or closed it) is not handed back. It wakes the waiting calls, which find the same page, tabs, and sign-ins.

A call the chat's Stop ends keeps the gate until its server answers, up to ten seconds, and is not called off: rmcp forgets a request it cancels while the server goes on with it. A call that answers in none of that time, or runs out of time, closes its browser, which ends any input it had left.

## Signing in with a saved secret

A bot that needs a password for a site asks for it on a card ([Secret requests](secrets.md)) and types it as `{{secret:NAME}}` with `browser_type` or `browser_fill_form`. The Runner fills it in right before the call reaches the browser, and only while the current tab is on the secret's site, over https; the page's echo comes back with the placeholder.

## Screenshots

A screenshot of a profile, the bot's `browser_take_screenshot` while its profile is open or Take Screenshot in the apps, is published as the bot's [output](outputs.md) in the chat through `outputs::publish`: one series per profile and chat, `Browser · <name>.png`, each screenshot its next version, with `after_screenshot` evidence that is `unverified` ("What the Work browser showed."). The PNG is queued as an encrypted `file` blob before the encrypted chat message, as any output's file is, and paired Devices open it from the transcript or the chat's outputs. Publishing the browser's own capture file needs no file Access. A screenshot shows the page; it says nothing on its own about whether a task worked.

## Methods and Devices

`browser.sessions { bot_id }` answers `{ sessions }`. `browser.create { bot_id, name }` answers `{ session }`; `browser.open`, `browser.takeover`, `browser.resume { revision }`, and `browser.stop` take `bot_id` and `session_id` and answer `{ session }`; `browser.delete` answers `{}`; `browser.screenshot { chat_id }` answers `{ message_id }`, for a chat the bot is in. The bot's Runner answers them all. Another Device sends them as sealed `request`s and waits up to 150 seconds, the time a browser takes to start or a call in flight to answer; a timeout leaves the outcome unknown, and the apps ask again.

| | On the bot's Runner | On another Device |
| --- | --- | --- |
| List, add, delete | yes | yes, through the relay |
| Open Browser | a window on this screen | no: it opens only on the Runner |
| Take Over, Return to Bot, Close | yes | yes; the user signs in at the Runner |
| Take Screenshot | yes | yes, what the Runner's browser shows |
| Watching or typing from afar | no | no |

None of these change the bot's Runner. Control of other applications on the Runner is not part of it.

## In the apps

The Browser plugin's sheet, opened from a bot's inspector, has a Profiles section for that bot (`Sheets/BrowserProfiles.swift` on the Mac, `desktop/browser_profiles.go` on Windows and Linux). A row per profile shows its name and how it stands: Closed, Open, Taking over…, or You have control in orange, while the bot waits on the user; its tooltip says what that means for the bot, and an action in flight reads Opening…, Closing…. The section's + names a new profile in an alert. A click or a right-click on a row opens its menu: Open Browser on the Runner, elsewhere a disabled "Open on <Runner>"; Take Over or Return to Bot; Take Screenshot, into the chat the inspector shows; Close Browser, also while an open or a takeover waits; and Delete…, which asks first. A failure is an alert with the Runner's words. With no profile yet, a note says what one is for. While the sheet is up it asks the Runner again every two seconds, five through the relay, and an answer begun before an action never covers the action's own. Opened from Settings, the sheet is the Runner's and has no Profiles section.

On the phone, which is never a Runner, Browser's row in a bot's Details slides in a Browser screen (`mobile/src/ui/BrowserScreen.tsx`) with the same rows and words, Add Profile as the section's last row (an alert that asks for the name), and the menu on a tap: SwiftUI's on iOS, labelled for VoiceOver with the row's words and state, Material's dropdown on Android, with Delete… in a group of its own. Its first item is the disabled "Open on <Runner>": opening a window and signing in happen at the Runner. The screen asks the Runner again every five seconds while it is up.
