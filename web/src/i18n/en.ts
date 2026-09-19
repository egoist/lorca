export const en = {
  meta: {
    title: 'Lorca',
    description:
      'A team of AI bots that live on your own machines. Direct chats, group chats where bots take turns, and an end-to-end encrypted relay between your devices.',
  },
  nav: {
    turns: 'Turns',
    relay: 'Relay',
    tools: 'Tools',
    faq: 'FAQ',
    download: 'Download for Mac',
  },
  hero: {
    badge: 'Now on macOS · Windows and Linux next',
    title: 'Bots that live<br/>on <accent>your</accent> machines.',
    body: 'A small team of AI bots, each on a machine you own. Talk to one, put a few in a group and let them take turns, hand work between them. Every message is encrypted before it leaves your computer.',
    how: 'See how it works',
    platforms: 'macOS 14 or later today · Windows and Linux on the way',
  },
  turns: {
    eyebrow: 'Group chats',
    title: 'Everyone gets a turn. <accent>Not everyone talks.</accent>',
    body: 'Post in a group and each bot is offered a turn, one at a time, in order. A bot answers when the message is for it or when it knows something the others need. Otherwise it passes and you never see a word. Rounds continue while anyone has something to add, then the room goes quiet. Mention a name to put that bot first; mention @everyone to hear from all of them.',
    alt: 'A Lorca group chat: Scout reports two leaks, Nova hands the schema change to Patch, Patch posts the migration.',
  },
  relay: {
    eyebrow: 'Your machines',
    title: 'The relay is a mailbox, <accent>not a reader.</accent>',
    body: "Your identity is a key pair made on your first machine; the backup is a phrase you write down. Pair the next computer with a string, and from then on chats sync through a relay that only ever holds ciphertext. When a bot on another machine has a turn, the job travels as an envelope sealed to that machine's key. Provider credentials travel the same way: connect a provider once, and your other machines get it encrypted with your account key, which the relay never holds.",
    alt: 'Three Macs exchange sealed envelopes through a relay that stores ciphertext only.',
    name: 'Relay',
    machines: ['Workbench', 'Studio', 'Closet mini'],
    ciphertext: 'ciphertext',
  },
  tools: {
    eyebrow: 'Real tools',
    title: 'Hands on the machine <accent>you chose.</accent>',
    body: 'Every bot has a working directory on its Runner and the same tools a coding agent gets: read, write, edit, grep, find, ls, and a shell. It runs as you, on the machine you assigned, and says what it ran. Each bot also keeps its own memory across chats, so the second time you ask, it already knows.',
  },
  chef: {
    eyebrow: 'Day one',
    title: 'Start with one bot. <accent>It hires the rest.</accent>',
    body: 'A new identity comes with Chef, a chief of staff. Chef asks what you work on, proposes a small team of one-job bots, and creates them when you agree. Rename it, replace it, delete it. Nothing about it is special except that it was there first.',
    steps: [
      { title: 'Create an identity', body: 'A key pair and a thirteen-group backup phrase.' },
      { title: 'Connect a provider', body: 'A DeepSeek key, or sign in to ChatGPT or Grok. It stays on this machine.' },
      { title: 'Meet Chef', body: 'Describe your week. Say yes to the team it proposes.' },
      { title: 'Pair the next machine', body: 'Paste the pairing string. Assign a bot to it.' },
    ],
  },
  faq: {
    title: 'Questions',
    items: [
      { q: 'Do I need a server?', a: 'No. One machine works on its own. The relay only matters when you pair a second device, and it stores ciphertext and nothing else.' },
      { q: 'Which platforms?', a: 'macOS today, on Apple silicon and Intel. Windows and Linux are next, and a bot on any of them can join the same team.' },
      { q: 'Which models can bots use?', a: 'DeepSeek with an API key, or ChatGPT or Grok by signing in with your own account. Each bot chooses its provider and model, and you can change them any time.' },
      { q: 'What can a bot do on my computer?', a: 'Read, write, and edit files, search, and run commands inside the working directory you give it. It runs as you, on the machine you assigned it to.' },
      { q: 'What does the relay see?', a: 'Encrypted blobs, a machine public key, and a sequence number. No names, no titles, no messages.' },
    ],
  },
  cta: {
    title: 'Give your machines a team.',
    body: 'On your Mac today. Everywhere you work, soon.',
  },
  footer: {
    privacy: 'Privacy',
    faq: 'FAQ',
    download: 'Download',
  },
}

export type Messages = typeof en
