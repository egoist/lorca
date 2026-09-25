# Changelog

The Mac app's release notes. `bun run release-mac` attaches a version's section to its update, and
Sparkle shows it in the update window.

## [Unreleased]

- Every command a bot runs shows in the chat as one card, from start to finish: Auto-review's question, if it asks, with Allow once, Always allow, and Deny; the command's latest output while it runs, in a block that scrolls; and one line when it ends.
- A bot's command that asks for input (a `sudo` password, an `ssh` passphrase, a `[Y/n]`) no longer hangs its turn. You answer or stop it from its card on any of your Devices. What you type goes straight to the command and is never saved in the chat, and once the command finishes, the bot hears how it went and carries on. A command that prints nothing for 20 seconds waits the same way, and one still waiting stops after 30 minutes without output, when Lorca quits, or when its chat is deleted.
- Command output reaches the bot without colors and other terminal codes, which stay in the full-output file.

## [0.1.0]

- The first release.
