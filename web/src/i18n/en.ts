export const en = {
  meta: {
    title: 'Lorca',
    description:
      'AI teammates that run on your own computer. Chat with one or several at once, let them work with your files, and they coordinate the work among themselves.',
  },
  nav: {
    turns: 'Group chats',
    relay: 'Privacy',
    tools: 'What it does',
    faq: 'FAQ',
    docs: 'Docs',
    download: 'Download for Mac',
  },
  hero: {
    badge: 'Now available for Mac',
    title: 'AI teammates<br/>for <accent>real</accent> work.',
    body: 'Chat with one teammate, or put several in a group chat. They can read and edit your files, run commands, and remember what you tell them. Give them a job and they coordinate the work among themselves.',
    how: 'See how it works',
    platforms: 'Also on Linux and Windows, from the command line.',
  },
  turns: {
    eyebrow: 'Group chats',
    title: 'Several bots in one chat.',
    body: 'Post in a group and each bot is offered a turn, one at a time. A bot replies when the message is addressed to it or when it has useful information to add. Otherwise it skips its turn. Mention a bot by name and it goes first.',
    alt: 'Lorca on macOS: a Researcher, Developer, and Project Manager prepare a launch together in a group chat across two Runners.',
  },
  relay: {
    eyebrow: 'Privacy',
    title: 'Encrypted on your computer.',
    body: "Add another computer or your phone and your chats sync to it. Data is encrypted on your device and sent through a relay. The relay has no decryption key, so it can't read bot names, chat titles, or messages.",
    alt: 'Encrypted data goes from your computer through the relay to your phone. Only your devices can decrypt it.',
    nodes: {
      computer: { title: 'Your computer', body: 'Encrypted before sending' },
      relay: { title: 'Relay', body: "Forwards encrypted data. Can't decrypt it." },
      phone: { title: 'Your phone', body: 'Decrypted with your key' },
    },
  },
  tools: {
    eyebrow: 'Tools',
    title: 'Files, commands, and the web.',
    body: 'Give a bot a folder on any of your computers. It works inside that folder, on that computer, and shows you what it ran.',
    kinds: {
      files: { title: 'Files', body: 'Opens, edits, and creates files in the folder you give it, and runs commands there.' },
      web: { title: 'Web', body: 'Searches the web and reads pages.' },
      memory: { title: 'Memory', body: "Keeps notes across chats, so you don't have to repeat yourself." },
      plugins: { title: 'Apps', body: 'Connects to GitHub, Notion, Linear, and other apps. Asks for permission first.' },
    },
  },
  chef: {
    eyebrow: 'Getting started',
    title: 'Start with one bot.',
    body: 'Every new account starts with one bot, Chef. Tell Chef what you work on and it suggests a few bots, each for one kind of task. Approve them and Chef creates them. You can rename or delete any bot later.',
    steps: [
      { title: 'Create your account', body: 'No email, no password. You get a backup phrase to write down.' },
      { title: 'Connect an AI provider', body: 'Sign in with ChatGPT or Grok, or paste a DeepSeek API key.' },
      { title: 'Talk to Chef', body: 'Tell it what you work on and approve the bots it suggests.' },
      { title: 'Add another computer', body: 'Paste a pairing code, then choose which bots run on it.' },
    ],
  },
  faq: {
    title: 'Questions',
    items: [
      { q: 'Do I need an account or a server?', a: 'No. Lorca runs on your own computer, with no sign-up and nothing to host. Your backup phrase is your account.' },
      { q: 'What does it run on?', a: 'Mac, with the app. Linux and Windows from the command line. A bot on any of them can join the same group chats.' },
      { q: 'Which AI does it use?', a: 'Your own account or API key. Sign in with ChatGPT or Grok, or add a DeepSeek API key. Each bot can use a different provider, and you can change it any time.' },
      { q: 'What can a bot do on my computer?', a: 'Read, edit, and create files and run commands in the folder you give it. It runs with your user permissions on that computer and shows you what it ran.' },
      { q: 'Can anyone read my chats?', a: 'No. Chats are encrypted on your computer before they sync. The relay stores only encrypted data and has no key, so it cannot read bot names, chat titles, or messages.' },
    ],
  },
  cta: {
    title: 'Get started.',
    body: 'Download for Mac, or use the command line on Linux and Windows.',
  },
  docs: {
    title: 'Lorca Docs',
  },
  footer: {
    privacy: 'Privacy',
    faq: 'FAQ',
    download: 'Download',
  },
}

export type Messages = typeof en
