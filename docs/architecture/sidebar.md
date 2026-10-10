# Sidebar sections and alerts

How chats are filed and quieted: sections in the sidebar, hidden chats, muted chats, and the chat a computer shows in front of the user, which no phone is alerted about. Sections, hidden chats, and mutes live in the roster, so every Device lists and alerts the same way; what each computer shows in front travels in its `machine` blob.

## Sections

The roster carries `sections: [{ id, name, collapsed }]` in their order, and each chat's metadata names the section it is listed under (`section_id`). Like the rest of the roster the list is latest-wins: two Devices editing sections at once keep the roster written last, as they do pins. A section's name is one line of at most 60 characters. The CLI keeps the sections in SQLite (`sidebar_sections`) and answers them in the snapshot and `roster.changed`.

The CLI's methods, from any Device:

- `sections.create { id?, name, chat_id? }` adds a section after the others and answers `{ section }`; `chat_id` moves that chat into it in the same roster change. An app mints the id so the header it shows at once is the one the roster brings back.
- `sections.rename { id, name }`, `sections.collapse { id, collapsed }`, `sections.reorder { ids }` (sections it leaves out follow, in their order), and `sections.delete { id }`, which leaves the section's chats in no section.
- `chats.set_section { chat_id, section_id | null }` lists a chat under a section, or with the chats in no section, where the sidebar shows it: the chat comes off the pinned rows and out of Hidden.

Every app lists the chats by the same rule. The pinned chats come first. Then each section's header with its chats that are neither pinned nor hidden, newest activity first; a section with no chats still shows its header, so a chat can be put in it. Then Chats (未分组), the chats in no section or in a section since deleted, shown only while it has any. Last comes Hidden, shown only while a chat is hidden. An account without sections sees one list with no header, as before sections existed.

A [channel's](channels.md) conversation, a chat of its own for each Telegram chat or Slack thread, starts in the section its bot's direct chat is in and with that chat's mute, so a bot filed and quieted once stays filed and quiet as people write to it.

A section folds on every Device: its header's fold control writes `collapsed`. Chats and Hidden fold on each Device alone, and Hidden starts folded. A chat opened from the palette, a notification, or search shows its row, so its group unfolds; folding a group around the chat on screen leaves the chat open.

## Hidden chats

`chats.hide { chat_id, hidden }` sets a chat's `is_hidden`: it leaves the sidebar's groups for Hidden but keeps its transcript, its unread count, and its turns. Hiding unpins it, and pinning a hidden chat or moving it to a section shows it again. Search still finds a hidden chat: the phone's search lists it, and the Mac's and the desktop app's command palettes list it once a query matches, as they do settings.

## Muted chats

`chats.mute { chat_id, muted, until? }` sets a chat's `mute`: `{ until }` in Unix seconds, or `{}` until it is unmuted. A chat is muted while its mute has no `until` or the time has not come, by each Device's clock; a mute that ran out stays in the roster and changes nothing.

A muted chat alerts nowhere. The Runner sends no phone push for it, whatever the push is about: a reply, a failure, a question (a permission card, a command's card, a [coding agent](coding-agents.md)'s card, a [secret request](secrets.md)), a coordinator's brief or urgent escalation (`push::should_notify`); the turns an event or a channel's message starts are no exception. Attention still lists what a muted chat raises, so muting quiets a chat without hiding what waits in it. The Mac app and the Windows and Linux app post no notification for it and leave it out of the Dock or taskbar badge. Its unread count is kept, and its row shows a crossed-out bell beside the stamp.

## The chat in front on a computer

A desktop app tells its CLI which chat the user has in front (`ui.watching { chat_id | null }`). The Mac app names one while it is frontmost with the chat in its main window, the screen is awake and unlocked, and the user gave some input in the last five minutes; it looks again every 15 seconds while it names one, and when the screen locks or sleeps. The Windows and Linux app names one while its main window has the focus. Either names none once it disconnects.

The CLI keeps the chat for itself (a reply that finishes there counts as read and clears on every Device) and, half a second after the app settles on it, puts it in its `machine` blob as `watching`, so a run through the sidebar goes up once. Every other Device keeps each computer's `watching` (SQLite `device_watching`, so a restart does not forget it). Before a Runner pushes, it checks that neither its own app nor any computer the relay lists online (`device_online`) has the chat in front (`App::is_watched_anywhere`), the same check it makes again after the three-second read grace and before each retry ([Notifications](notifications.md)). A computer the relay lists offline stops counting, whatever its last blob said.

## In the apps

**macOS** (`Sidebar/`): the sidebar's `NSOutlineView` lists a section, Chats, and Hidden as group rows (`SidebarLayout` in `SidebarRows.swift`), whose Show and Hide control AppKit shows on hover, and keeps its rows incremental across groups: a chat moving between groups is one move. A chat's context menu holds Pin, Mute (For 1 Hour, For 8 Hours, For 1 Week, Always) or Unmute, Move to Section (each section, checked where the chat is, then Chats, then New Section…; Move to New Section… while there are none), and Hide or Show in Sidebar; the Chat menu has the same items for the chat on screen, File has New Section…, and the palette offers Unmute Chat, Hide Chat, and New Section…. A section's header renames, moves up or down, and deletes it (after asking: its chats move to Chats), and the Chats header starts a section. Chats drag onto a group, and sections drag between sections. Names are asked for in an alert with a field (`SectionPrompt`). ⌘1–⌘9 count the chat rows that show.

**Windows and Linux** (`sidebar.go`): the same rows from `sidebarRows`, with a header's chevron at its trailing edge while the pointer is over it, the same menus, a menu bar rebuilt when the sections or the chat's place among them change, and rows that drag into a group or, for a section's header, among the sections (`ui.ListState.Reorder`).

**Phone** (`src/ui/chatList.ts`, `ChatsScreen.tsx`): the chat list's group headers are iOS's collapsible list sections (a bold title and a tinted chevron) or Material subheaders; a tap folds one, and a long press on a section's renames, moves, or deletes it. A row's menu (the iPhone's peek menu, the sidebar's and Android's long-press menu) adds Mute or Unmute, Move to Section, and Hide or Show in Sidebar as submenus. Details has Mute, Section, and Hidden rows beside Pinned, and the New menu has New Section. Names are asked for in UIKit's alert with a field, or a Material dialog with one on Android (`prompt` in `src/ui/alert.tsx`).
