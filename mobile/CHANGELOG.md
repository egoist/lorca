# Changelog

The phone app's release notes, by the version in `app.config.ts`. A release (a `mobile-vX.Y.Z`
tag) takes a version's section as the notes of its GitHub release, and the Android app shows them
when it offers the update.

## [Unreleased]

- A bot hands a coding agent your request in your words, with only where the repository is and what
  you already said, and no plan, checklist, branch, or way to check it of its own. The agent decides
  how to work, pushes, opens a pull request, and reports to you with the commands it ran and what
  they printed. Its report notifies you as a reply does; the bot no longer checks its work or
  follows up on its own, and speaks up only when the agent ends on its own.

## [1.0.8]

- Coding agents are set per Runner: each Device keeps its own agents, in its own order and each on
  or off, and shows which are installed on it.
- A bot connected to Gmail can send an email itself, not only leave a draft. With Draft first on,
  it still waits as a draft card until you press Send.

## [1.0.7]

- Coding agents work the way Grok Bot's do. A bot hands a job to Claude Code, Codex, OpenCode,
  pi, or fx under a short title, and the chat shows "Messaged" and "Message from" markers with the agent's
  logo instead of a card. Tap one to follow the agent's work: what it thought and said, the files it
  explored, the commands it ran, and its edits. The chat's details list the agents under Coding.
- Each Runner in Settings has its Coding Agents: turn each on or off there, see whether it is
  installed, and drag one to change which its bots try first.
- A bot no longer shows as working after its turn ended when the Device it runs on drops off the
  relay: the working row and the dot on its avatar clear at once, without your next message.
- Gmail, Google Calendar, and Google Drive work over Google's own APIs. Your Google Cloud project
  needs only the service's API turned on, with no Workspace Developer Preview enrollment.
- The model list adds OpenCode's free and preview models and new Zen and Go models, and DeepSeek V4
  Pro shows its current price.

## [1.0.6]

- Long-press a brief in Attention and choose Dismiss to take it off the list on every Device; the
  coordinator's next brief shows again. A brief also leaves once every item it is about is resolved.
- A bot resolves an Attention item it alone reported as soon as it sees the work is done, such as a
  run it approved or a pull request that merged, so you no longer clear them by hand.
- You can scroll up a chat while a reply comes in, without being pulled back to the newest
  message.
- Several outputs in a row from one bot, such as the proof files a coding agent saves, show as one
  list with a line each.
- Auto-review allows an action by default, and holds only one it can name a rule for. A held action
  goes back to the bot before it reaches you.

## [1.0.5]

- Conversations a Telegram or Slack channel keeps show the service's real mark on their avatar
  badge.
- Fixed a crash on iOS when a keyboard that had already settled left the chat's composer inset with
  no height.

## [1.0.4]

- A chat that asks for a channel gets one reply, "Pending approval", and once you allow it, the bot
  answers what people asked while it waited. A chat that never asks can be added by its id with
  Add Chat… on the channel's screen.
- Conversations a Telegram or Slack channel keeps show the service's mark on their avatar, so they
  read apart from the bot's own DM.

## [1.0.3]

- A channel answers only in the chats you allow. Anyone can message a Telegram bot or add it to
  their group, so a chat you haven't allowed now asks first: the channel's row says A chat is
  waiting, and its screen lists it with Allow and Ignore. Until you allow it, the bot does nothing
  there. A channel that listened in every chat its bot was in now listens nowhere until you allow
  its chats, and each chat asks once someone writes there.
- Tapping an item in the Attention list that waits on a review opens its chat with the review on
  top, so you can approve or reject it right away.

## [1.0.2]

- A Telegram channel that listens for mentions or hashtags tells you when it can't hear them. While
  the Telegram bot's privacy mode is on, Telegram sends it only replies and commands in groups. The
  channel's row in a DM's Details now says Privacy mode on, and its screen says what it misses and
  how to turn privacy mode off.
- On Android, a message you send rests below the chat bar while the reply comes in, instead of
  scrolling up under the status bar.
- On Android, Install Update shows the download's progress in a dialog until the app restarts, and
  says so when the download or install fails.

## [1.0.1]

- Update Lorca on every Device: the phone app, the Mac app, the Windows and Linux app, and each
  computer that runs the command line. Devices now sign what they send each other through the relay,
  so a relay can no longer send work to your Runners in another Device's name, and unpairing a
  Device gives your account a new key, so the Device you unpaired cannot read what your account
  writes after that. An older Lorca cannot read what an updated Device sends, and once your Devices
  have updated, Lorca's relay asks older versions to update before they sync.
- A bot's Access changes from the phone: the Access row in a DM's Details, or Edit Access… on a
  bot's request for more access. Each change saves at once.
- A bot writes an email or a Slack message as a draft card in the chat, editable in place, and
  nothing goes out until you tap Send. What a routine leaves for your review waits in the chat the
  same way.
- What bots publish shows as a card in the chat, and Details lists a chat's outputs and tasks. New
  Task in Details adds a task.
- A secret a bot asks for goes on a card you fill in from the phone, sealed to the bot's Runner; it
  never enters the chat. The Runner's screen in Settings lists its secrets, without their values.
- Tapping a routine opens its own screen: how it stands and how to fix it, Run Now, Pause or Resume,
  Limits, its schedule and timezone, and missed runs. A routine that needs you says so in Details.
- Sidebar sections, and Hide and Mute in a chat's menu. A muted chat sends no push, and neither does
  the chat you have in front on a computer.
- Paste an image into the message box.
- Give Feedback in a DM's Details, project context in a group's Details, a bot's channels, browser
  profiles, and coding agents' cards with their transcripts, as on a computer.
- Share a bot as a template link, and open one to make a bot from it.
- Any Device renames any Device: Rename… on its screen in Settings.

## [1.0.0]

- Lorca for Android is on GitHub, and keeps itself up to date: once a day it looks for a new
  release and offers to install it, and Settings › Updates checks at once.
