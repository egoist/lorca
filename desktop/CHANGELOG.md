# Changelog

The Windows and Linux app's release notes, by the version in `desktop/package.json`. A release
(`bun run release-desktop`, or a `desktop-vX.Y.Z` tag) attaches a version's section to its update,
and the update window shows it.

## [Unreleased]

- Pairing a computer or restoring your identity no longer asks you to connect a provider your
  account already has. Onboarding waits until the account's providers arrive from the relay, which a
  slow connection or a long list of chats used to outlast.
- Onboarding's last step says the computer is paired, or that your identity is restored, instead of
  calling it your first Device. A paired computer is no longer pointed to Pair a Device, which only
  the computer that created or restored your identity can do.
- While onboarding pairs or restores, the Pair or Restore button and the field are disabled beside a
  spinner, and Back stops a pairing that is still waiting on the other computer.
- The marketplace adds plugins for 飞书, 飞书项目, 滴答清单, 腾讯文档, 秘塔 AI 搜索, 知乎, 高德地图, and
  可灵. 飞书, 飞书项目, 滴答清单, 腾讯文档, and 可灵 sign in with your account in the browser; 秘塔 AI
  搜索 and 知乎 take an API key from their sites, and 高德地图 a Web Service key from the Amap console.
- On Windows 10 the main window and onboarding no longer show Windows' own title bar, with a second
  set of window buttons, above the app's.
- Settings › Devices can unpair this computer too: Lorca forgets the account's keys, credentials,
  and chats here and goes back to onboarding. When this computer holds your identity, the
  confirmation says that coming back, or pairing a new Device, takes your backup phrase.
- Red spelling underlines no longer appear under API keys, URLs, plugin variables, names, and
  searches as you type them, or in the description, rule, and memory sheets. The composer still
  checks spelling.
- Bot avatars, image attachments, and the app icon can no longer be dragged out of the window.

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
