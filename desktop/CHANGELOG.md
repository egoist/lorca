# Changelog

The Windows and Linux app's release notes, by the version in `mygo.config.ts`. A release
(`bun run release-desktop`, or a `desktop-vX.Y.Z` tag) attaches a version's section to its update,
and the update window shows it.

## [Unreleased]

- A bot no longer shows as working after its turn ended when the Device it runs on drops off the
  relay: the working row and the dot on its avatar clear at once, without your next message.

## [0.1.12]

- Right-click a brief in Attention and choose Dismiss to take it off the list on every Device; the
  coordinator's next brief shows again. A brief also leaves once every item it is about is resolved.
- A bot resolves an Attention item it alone reported as soon as it sees the work is done, such as a
  run it approved or a pull request that merged, so you no longer clear them by hand.
- A bot listening on Telegram sees the photos people send it, with a caption or without one, and an
  album's photos together as one message. The photo shows in the conversation on every Device.
- Several outputs in a row from one bot, such as the proof files a coding agent saves, show as one
  list with a line each.
- Auto-review allows an action by default, and holds only one it can name a rule for. A held action
  goes back to the bot before it reaches you.

## [0.1.11]

- While a bot's turn runs for a message a Telegram channel took, that chat shows the bot typing, and
  the reply stops it.
- A custom provider's model offers the thinking levels its own list declares, `xhigh` and `max`
  included, and sends them as written.
- Every bot has a built-in `lorca` skill that points at the docs on lorca.app, so it looks up how
  Lorca works instead of guessing.

## [0.1.10]

- A chat that asks for a channel gets one reply, "Pending approval", and once you allow it, the bot
  answers what people asked while it waited. A chat that never asks, such as a group where the bot
  only hears replies to it, can be added by its id with Add… on the channel's Chats row.
- Conversations a Telegram or Slack channel keeps show the service's mark on their avatar, so they
  read apart from the bot's own DM.

## [0.1.9]

- A channel answers only in the chats you allow. Anyone can message a Telegram bot or add it to
  their group, so a chat you haven't allowed now asks first: the channel's row says A chat is
  waiting, and its sheet lists it under Asking to Chat with Allow and Ignore. Until you allow it,
  the bot does nothing there. A channel that listened in every chat its bot was in now listens
  nowhere until you allow its chats, and each chat asks once someone writes there.
- Clicking an item in the Attention list that waits on a review opens its chat with the review on
  top, so you can approve or reject it right away.

## [0.1.8]

- A Telegram channel that listens for mentions or hashtags tells you when it can't hear them. While
  the Telegram bot's privacy mode is on, Telegram sends it only replies and commands in groups. The
  channel's row in the bot's Details now says Privacy mode on, its sheet says what it misses and how
  to turn privacy mode off, and the bot tells you when it sets up or changes such a channel.

## [0.1.7]

- Update Lorca on every Device: the Windows and Linux app, the Mac app, the phone app, and each
  computer that runs the command line. Devices now sign what they send each other through the relay,
  so a relay can no longer send work to your Runners in another Device's name, and unpairing a
  Device gives your account a new key, so the Device you unpaired cannot read what your account
  writes after that. An older Lorca cannot read what an updated Device sends, and once your Devices
  have updated, Lorca's relay asks older versions to update before they sync.
- Each bot has Access, which only you change: how far it may use each plugin (Read only, Read and
  draft, Read and write, or No access), which of its tools it may call, and whether it may read or
  change files and run commands on its Runner. When a bot reaches a limit, the chat offers Edit
  Access… to change it. A bot you never set keeps full access.
- A bot writes an email or a Slack message as a draft card in the chat, with its recipients,
  subject, text, and attachments editable in place, and nothing goes out until you press Send.
  Access › Messages turns drafts off for a bot, as Always Send does on a Slack card.
- Routines no longer refuse what Auto-review would ask you about: they leave it for your review. The
  chat says so, the inspector lists what waits, and Approve runs exactly what the sheet shows, edits
  included.
- Bots publish what they make, such as a report, a screenshot, or a chart, with what they checked:
  Passed, Failed, or Not verified, and the command they ran. An output shows as a card of its own in
  the chat, and the inspector lists a chat's outputs.
- Tasks: work that outlasts one turn, with an owning bot, what done looks like, the next step, and
  the evidence once it's done. A chat with tasks has a Tasks section in the inspector, and File ›
  New Task… adds one.
- A bot that hands work to a teammate gets the result back in its chat when the teammate finishes,
  gets stuck, or is stopped, also from a Runner that was offline when it asked.
- One list of what needs you: the coordinating bot keeps the reviews, blockers, and promises your
  bots report, with a short brief, behind a toolbar button that appears while something waits.
- A group chat keeps a shared project context beside its transcript: a brief, goals, constraints,
  decisions, facts, links, and files, which every bot in the group reads. It's the Project section
  of the group's inspector.
- Skills: Save as Skill… on a bot's reply, or Save as Standing Instruction… on your own message,
  writes a reusable instruction from the messages you pick, and it is used once you save it. Give
  Feedback… on a bot's message teaches the bot, which suggests changes to its routines and skills
  for you to accept.
- Workflows set up an outcome in one place, such as meeting preparation, inbox triage, or watching a
  repository: the bot, the accounts it needs, and a sample run before it runs on a schedule. They're
  first in the marketplace, and on onboarding's last page.
- Share a bot as a link: File › Share as Template… shows exactly what the template holds, keys and
  passwords removed, and the link opens a page that shows the bot and opens it in Lorca. Neither the
  relay nor lorca.app can read it.
- Limits on a bot's turns, a routine's runs, and a task: spending, tokens, run time, retries, and
  plugin calls, with Resume when work stopped at one. Each plugin account has a call limit all bots
  share.
- Browser profiles keep a bot's sign-ins on its Runner. Take Over gives you the bot's browser and
  Return to Bot hands it back, and Record teaches the bot a browser workflow by doing it once.
- Gmail, Google Calendar, Google Drive, and Slack are in the marketplace, each with as many named
  accounts as you want, such as a Work Gmail beside a Personal one.
- A bot asks for a password, a key, or a code on a card, and the value goes sealed to its Runner. It
  never enters the chat or the model. Settings › Secrets lists them, without their values.
- Routines keep their timezone, run once when their Runner comes back from missed times (or skip
  them), and say when they need you. A bot can also set a one-time reminder, watch a pull request,
  run around your calendar's events, or run on a webhook, hosted on Lorca's relay.
- A bot can listen on a Telegram or Slack account and answer people in their thread. The Feedback
  collector workflow files community feedback as GitHub issues.
- A bot can run Claude Code or Codex on its Runner and supervise it, and the chat shows the run as a
  card with its transcript and its questions.
- Sidebar sections, and Hide and Mute in a chat's menu. A muted chat sends no notification and stays
  out of the taskbar badge, and the phone gets no push for the chat you have in front on a computer.
- Stop also stops the work a turn handed off, and ends a turn within about 15 seconds whatever its
  tools do.
- Any Device renames any Device: Rename… in its Settings › Devices pane.
- A message you send while a bot waits on a background command, or on a browser you took over,
  reaches the bot at once.
- On a Windows Runner, a bot's commands run in a terminal, so they can go to the background, take an
  answer, and show in Running tasks, and Stop ends one that held the turn.
- Claude Haiku 5.5, and Sonnet 5.5's lower cache read price.
- Only an app on this computer that holds Lorca's token can connect to its command line.
- Typing with an input method, such as pinyin, keeps an @mention in the message box styled.
- Settings › Providers has Review Models: pick the model Auto-review runs on each connected
  provider instead of its small, fast default, including a decision model such as OpenCode Zen's
  Jev 1.13, which only allows an action or names what it could harm. Settings › Auto-review has
  Reviews with, to review with another connected provider than the bot's.
- Add Provider… offers decision APIs for Auto-review: OpenRouter Decisions, OpenAI Decisions, and
  TypeSafe, and a custom provider can speak System One or OpenAI Decisions. Bots never run on
  them.

## [0.1.6]

- On Linux, double-clicking the top of the main window maximizes it or restores its size, as a
  title bar does; on Wayland it did nothing.
- On Linux, small capitalized headings and pairing codes show in full: their letter spacing made
  them wrap or cut off.
- The message box's attach button and the search field's Clear button show the arrow pointer, not
  the text cursor.

## [0.1.5]

- Open Lorca, on onboarding's last step, opens the main window; before, the app quit there.

## [0.1.4]

- A computer that runs the Lorca command line without the app keeps it up to date by itself: it
  installs each new release, signed by Lorca, and restarts into it once no bot is working there.
  Settings › Devices shows the version it runs, and Update when a newer one is out, so you can
  update it from this computer. `lorca service install` keeps `lorca serve` running on such a
  computer, from login on.

## [0.1.3]

- The marketplace comes from lorca.app, so new plugins and bots show up without an update, and
  plugins you installed from it get their fixes the same way. Lorca checks when it starts, when the
  app connects, and when you or a bot look through the marketplace, at most once an hour, and a bot
  that can't find a plugin you asked for checks again first. `lorca marketplace reload` checks
  right away.
- A bot's scripts can run commands, edit files, and use its memory, as pi's do, not just read and
  write files and call plugins. A script can run a command for each project or file it finds, keep
  going when one fails, and hand the bot only what matters. Auto-review checks each command with
  the script that runs it, and one that needs your permission asks in the chat.
- While a bot's script runs a command, the working row says which, as it does for a command the
  bot runs itself, and bots can read a plugin's whole instructions from its server, not just their
  start.
- Window titles end with ` - Lorca`, so the taskbar and Alt+Tab say which app a chat belongs
  to.

## [0.1.2]

- Your own MCP servers: Settings › Plugins has an MCP Servers section, where you add a server by
  the command that runs it or its URL, or paste its JSON from a README or another app's settings,
  and edit, turn off, or remove it. Each server shows how it stands and the tools it offers, with
  Sign in when it asks for one, and every bot on that computer can use it. Lorca keeps them in
  `mcp.json` in its folder, in the format Claude Desktop, Cursor, and Claude Code use; after
  editing the file by hand, click Reload. Servers that offer resources, such as files or records,
  give bots tools to list and read them. Pick another Runner in Settings to manage its servers.
- MCP servers take a `timeout` for slow tools; can keep tools from bots, with a switch beside each
  tool on the server's sheet or `toolExposure` in mcp.json, as pi writes it; and can sign in where a
  server wants an app registered with it, with the redirect port or URL it was registered with, a
  name to register under, and the authorization server's address when the server names the wrong
  one. A server that needs more access asks you to sign in again for it.
- `lorca mcp` lists, adds, removes, turns on or off, hides tools of, reloads, and signs in to or out
  of MCP servers from a terminal, and `lorca mcp import` adds the servers Claude Desktop, Claude
  Code, Cursor, Windsurf, VS Code, or Gemini CLI have on the computer.
- Bots can add an MCP server the marketplace lacks, as its README gives it: the `lorca` command is
  in their shell, pointed at their Runner, and Auto-review checks each change it makes, asking you
  when you didn't ask for it.
- Plugin servers are sturdier. One that fails to start says why, in the words it printed, and one
  that stopped starts again on the next call instead of failing until Lorca restarts. Stopping a
  server also stops what it started, such as npx's node; a call you stop is called off at the
  server; servers start side by side, so a slow one holds up no other; a server's new tools reach
  bots without a reconnect; and a remote server that is busy for a moment is tried again. A server
  whose tool list never ends no longer hangs while it connects, and an image a tool returns that no
  model takes, such as an SVG, is named instead of failing every later turn of the chat. A bot that
  calls a tool by a wrong name hears the closest right ones. A sign-in is never sent to another
  host when a server's address changes, and a plugin's sheet now has Sign Out beside Sign in again.
- Custom providers: Add Provider… in Settings' Providers pane adds OpenAI, OpenRouter, Gemini,
  Groq, Together AI, Ollama, LM Studio, or any other server that speaks OpenAI's Chat Completions
  or Responses API or Anthropic's Messages API, such as a gateway or a model server on your
  network. The sheet loads the models the server lists to pick from, and takes any it does not
  list. Bots pick the provider, its models, and a thinking level as they do a built-in one, and it
  reaches your paired Devices encrypted with the account key.
- Bots on ChatGPT, Grok, and OpenCode's GPT, Grok, and Muse Spark models see the images their
  tools return, such as a browser plugin's screenshot or an image file they read. They used to get
  only the text beside the image.
- Bots see the images you attach and their tools return in a form every model takes. A BMP, a
  GIF, or a TIFF is converted, a photo taken sideways is turned upright, and a large photo or
  screenshot is scaled down to fit 2,000 pixels, with a note telling the bot its original size.
  An image that still cannot go, such as an SVG, or a HEIC photo, which only a Mac converts, is
  named with the reason. Before, such an image could make every later message in its chat fail.
  Each message sends a chat's latest 20 images, so a long chat full of screenshots keeps working,
  and a text file that starts with "BM" now reads as text.
- Pairing a computer or restoring your identity no longer asks you to connect a provider your
  account already has. Onboarding waits until the account's providers arrive from the relay, which a
  slow connection or a long list of chats used to outlast.
- Onboarding's last step says the computer is paired, or that your identity is restored, instead of
  calling it your first Device.
- Pair a Device works on every paired computer. On a computer that had joined by pairing it showed
  an error, since only the computer that created or restored your identity could pair others. If you
  run your own relay, update it first.
- Settings › Devices shows a machine that is paired to your account but never sent its name or
  system as Unknown Device, with a note to unpair it if you don't recognize it.
- While onboarding pairs or restores, the Pair or Restore button and the field are disabled beside a
  spinner, and Back stops a pairing that is still waiting on the other computer.
- The marketplace adds plugins for 飞书, 飞书项目, 滴答清单, 腾讯文档, 秘塔 AI 搜索, 知乎, 高德地图, and
  可灵. 飞书, 飞书项目, 滴答清单, 腾讯文档, and 可灵 sign in with your account in the browser; 秘塔 AI
  搜索 and 知乎 take an API key from their sites, and 高德地图 a Web Service key from the Amap console.
- On Windows 10 the main window and onboarding no longer show Windows' own title bar, with a second
  set of window buttons, above the app's.
- Settings › Devices can unpair this computer too: Lorca forgets the account's keys, credentials,
  and chats here and goes back to onboarding. When this computer holds your identity, the
  confirmation says that your backup phrase becomes the only way to restore it.
- Red spelling underlines no longer appear under API keys, URLs, plugin variables, names, and
  searches as you type them, or in the description, rule, and memory sheets. The composer still
  checks spelling.
- Bot avatars, image attachments, and the app icon can no longer be dragged out of the window.
- On Windows, plugins that start with `npx`, such as Browser and 高德地图, no longer fail with
  "Cannot start npx: program not found". Lorca finds a plugin's program as a terminal does, so it
  finds the `npx.cmd` that Node.js installs.

## [0.1.1]

- A routine can watch for something without spending a turn each time: the bot gives it a check, a
  short script that looks at an inbox, a repository, or a feed at each due time and starts the bot
  only when it finds something new. Checks only read and never change anything. A check that finds
  nothing runs no turn, so it spends nothing on the bot's model; one that asks the small model to
  sort or screen what it read pays for those calls, which count in the chat's Spent figure. A
  routine's sheet shows its check, and the inspector its next check.
- When a long chat fills a bot's context, the summary that replaces the older part, and the memory
  save before it, reuse the prompt cache of the bot's own turn instead of sending the whole chat
  again, so compacting while a bot works costs a fraction of what it did.
- Lorca opens on your last chat, even when you quit it with Settings open.
- Toggle Inspector shows the inspector in a window too narrow for it. Opening the inspector or the
  sidebar where there is no room widens the window by the pane; a maximized window keeps its size,
  and the chat narrows instead.
- On Linux without a tray icon, finishing onboarding opens the main window, and Delete Account
  opens onboarding. Before, Lorca quit.
- Your phone still gets the notification for a reply that finishes while the relay restarts or is
  briefly out of reach.

## [0.1.0]

- The first release.
