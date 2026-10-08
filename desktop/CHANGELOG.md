# Changelog

The Windows and Linux app's release notes, by the version in `mygo.config.ts`. A release
(`bun run release-desktop`, or a `desktop-vX.Y.Z` tag) attaches a version's section to its update,
and the update window shows it.

## [Unreleased]

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
