# Secret requests

A password, an API key, or a one-time code a bot needs in the middle of a task: the bot asks on a card in the chat, the user answers there from any Device, and the value goes sealed to the bot's Runner and stays there. It never enters the transcript, the synced chat, or the model's context; the bot uses it by name. `crates/cli/src/secrets/` holds the tool, the store, the fill, and the scrubbing.

## Asking

`request_secret { use, why, site?, plugin?, secrets: [{ name, label }] }` says where the values go (`use`), why the bot needs them, and one to five values, each by the name the bot uses (letters, digits, and `_`, not starting with a digit, unique on the card) and the label the card shows ("GitHub password"):

- `browser`: typed into a sign-in page of `site` in the bot's Browser. `site` is a host (`https://www.github.com/login` is `github.com`). Browser must be installed on the Runner and in the bot's [Access](bot-permissions.md).
- `command`: an environment variable of the bot's commands, the name being the variable. Shell commands must be in the bot's Access, and a name the shell runs by (`PATH`, `HOME`, `SHELL`, `USER`, `TERM`, …, or one starting `LORCA_`, `DYLD_`, `LD_`) is refused.
- `plugin`: a setting of an installed `plugin` the bot's Access allows, the name being one of the variables its manifest declares.

The tool posts a `permission` message with `tool` `secret`: `summary` is the labels, `reason` the bot's why, and `secret` carries `{ use, site?, fields: [{ name, label }] }` beside `plugin_id` and `plugin_name` (`playwright` and Browser, the plugin, or `computer` and the Runner's name). The call waits for the answer as a permission card's does (`mcp::await_answer`): up to ten minutes, and a new message from the user dismisses it. [Stop](stopping.md) ends it too: the card reads Dismissed, nothing is kept, and the bot hears that the turn was stopped before the user answered, not that the user refused. The bot hears that the user saved the values and how to use them, that the user chose Not now (`denied`), that nobody answered (`expired`), or that the user wrote instead (`dismissed`). A routine's or an event's turn has nobody to answer and cannot ask. A pending card counts as unread and pushes "Asks for npm token" to the phones, as any question does.

## Answering

The card's Save sends `chats.permission { chat_id, message_id, decision: "allow", values: { NAME: value } }`. The Device that answers passes it to its CLI, which serves it there when it is the bot's Runner and otherwise seals it to the Runner's box key as a `permission.answer` [request](protocols.md#cli--relay); the relay holds only that ciphertext until the Runner consumes and deletes it, and the response carries no value. Not now is `decision: "deny"`.

On the Runner the values count only while the card still waits: one for a card nobody waits on (answered, expired, or dismissed) is refused and kept nowhere. Each value is trimmed and must hold 4 to 8,192 characters, so it can be told apart wherever a tool echoes it. A `browser` or `command` value goes into the Runner's store, a `plugin` value into the plugin's own settings (`plugins::set_variables`, its `secrets.json`), and only then does the waiting call hear the answer; the card reads `allowed` (Saved). No value is written to the card, a chat row, a push, or a log.

## The store

The Runner keeps its secrets in `secrets.enc` in its data directory, encrypted with the account DEK (`secrets` as associated data), written to a private file and renamed into place, and reads it once per account. A secret is `{ id, bot_id, name, label, use, site?, value, updated_at }`. It belongs to the bot that asked: a bot uses only its own, and asking again for a name it has replaces that secret. Deleting the bot, here or on another Device, deletes its secrets; forgetting the identity deletes the file. A bot moved to another Runner finds none of them there and asks again.

The system prompt names the bot's saved secrets and where each goes ("github_password (Browser on github.com), NPM_TOKEN (commands)"), never a value.

## Using a secret

- **Browser.** The bot writes `{{secret:NAME}}` in `browser_type`'s `text` or a `browser_fill_form` field's `value`. The Plugin call goes through Access and [Auto-review](tools.md) with the placeholder, and right before the request leaves for the server (`PluginTool::execute`, after the call limits) the Runner asks the browser for its tabs (`browser_tabs`) and fills each placeholder only when the current tab is on the secret's site or one of its subdomains, over https (plain http only on `localhost`, `127.0.0.1`, or `[::1]`). On another page, or with a page it cannot read, the call fails saying so and nothing is typed. A placeholder anywhere else in Browser's arguments (an address, an element's name, a script) or in any other plugin's call is refused. It works in a [browser profile](browser-sessions.md) or the shared headless browser alike.
- **Commands.** `bash { …, secrets: ["NAME"] }` sets `$NAME` for that one command, in a terminal or on pipes, a codemode script's `bash` included (`lorca_agent::tools::SecretVariables`, `CommandSecrets`). Only the bot's own `command` secrets are given; any other name fails the call. A command that names secrets never takes Auto-review's read-only fast path: the review reads which variables it holds and is told that sending one anywhere but the service it is for leaks it.
- **Plugin settings.** The plugin uses the value as any of its variables, and its sheet replaces it.

## Keeping values out

Every tool result passes through the turn's `after_tool_call` hook (`secrets::scrub_result`) before the model, the call's chat row, or a codemode script sees it: in its text, its details, and its structured output, each saved value on the Runner becomes its placeholder, in the spellings a tool echoes one in: as typed, escaped in a JSON or a JavaScript string (Playwright writes the code it ran, `fill('…')`), and percent-encoded. A command card's output, question, and outcome are cleared the same way before the card goes up (`shell::put_up`), so Running tasks and every Device show the placeholder too. A routine's check, and a watch's or a calendar's [read](routine-triggers.md), runs its calls outside a turn (`routines::CheckRunner`), and their results are cleared the same way before the script, the run it starts, or the roster sees them; an event's payload and what started a routine's run are cleared again as the turn reads them. A browser recording keeps a saved value only as its placeholder, and a run of a skill's recorded steps types one as the bot's Browser calls do ([Recording a workflow](browser-sessions.md#recording-a-workflow)), so a skill names secrets and holds no value.

A bot's email or Slack message that carries a saved value or a placeholder is never [drafted](drafts.md), so neither reaches a draft card or the review item behind it, and a [channel's](channels.md#conversations) reply that carries one is never sent to Telegram or Slack; what someone there writes is kept with the values cleared and its placeholders broken. What stays as it was: the raw bytes a long command output spills to a temp file on the Runner, and a plugin setting's value, which reaches the plugin's server as its other settings do. A bot with shell or file access runs as the Runner's user, who can read the store and the account key: as with plugin secrets, the stronger separation is a Runner that holds only what that bot may reach ([Bot permissions](bot-permissions.md#shell-filesystem-and-isolation)).

## Managing a Runner's secrets

`secrets.list { runner_id }` answers `{ secrets: [{ id, bot_id, name, label, use, site?, updated_at }] }`, never a value; `secrets.set { runner_id, id, value }` replaces one and `secrets.delete { runner_id, id }` deletes one. The Runner answers them, for another Device as sealed `secrets.*` requests, so a new value travels only as ciphertext.

## In the apps

On the Mac a card asking for a secret is `Chat/SecretCellView.swift`: "Developer needs a secret for github.com" (or "for GitHub", "for its commands"), why, a field for each value that hides what is typed (Return moves to the next empty one, then saves), Save and Not now, and "Saved on Studio. Developer never sees it." in a line that a save that fails takes over to say why, so the transcript never moves. Answered, it reads "Saved · npm token". Settings › Secrets (`Settings/SecretsSettingsViewController.swift`) shows the picked Runner's secrets, asked when the pane shows a Runner and after each change made there: each row is the bot's avatar, the label, and "Developer · github.com" or "Researcher · Commands"; a click opens the secret's sheet (`Sheets/SecretViewController.swift`: a field for a new value, Replace, and Delete… apart), and its menu has Replace… and Delete…. With none the section says so, and the footnote says how secrets get there.

The Windows and Linux app has the same card (`secretCard` in `desktop/cards.go`) and pane (`desktop/settings_secrets.go`).

On the phone the card (`SecretRow` in `mobile/src/ui/transcript.tsx`) has Fill In, which opens a bottom sheet with a field for each value and Save beside the last (`SecretSheet`, `@expo/ui`'s on both platforms, as a command's answer sheet), and Not now. A Runner's screen in Settings has a Secrets section while it keeps any, each row's menu replacing or deleting one (`RowMenu`).
