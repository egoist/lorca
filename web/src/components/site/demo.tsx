import { useEffect, useState } from 'react'

/// The hero's live group chat: three bots take turns after a message, in the app's own
/// vocabulary (a working row, a pass that says nothing, a handoff into a DM). Loops.
type Bot = { name: string; hue: string; glyph: string }

const bots: Record<string, Bot> = {
  nova: { name: 'Nova', hue: 'from-violet-400 to-indigo-500', glyph: '✦' },
  scout: { name: 'Scout', hue: 'from-teal-300 to-cyan-500', glyph: '◎' },
  patch: { name: 'Patch', hue: 'from-sky-300 to-blue-500', glyph: '</>' },
}

type Line =
  | { kind: 'you'; text: string }
  | { kind: 'bot'; bot: string; text: string }
  | { kind: 'working'; bot: string; activity?: string }
  | { kind: 'chip'; bot: string; text: string }

type Frame = { lines: Line[]; hold: number }

const script: Frame[] = [
  { lines: [], hold: 900 },
  { lines: [{ kind: 'you', text: 'The relay still stores chat titles in plaintext. Who owns this?' }], hold: 700 },
  { lines: [{ kind: 'working', bot: 'nova' }], hold: 1400 },
  {
    lines: [
      { kind: 'bot', bot: 'nova', text: 'Titles belong inside the roster blob. Scout, find every place one leaks; Patch takes the schema.' },
      { kind: 'working', bot: 'scout', activity: 'Searching files' },
    ],
    hold: 1600,
  },
  {
    lines: [
      { kind: 'bot', bot: 'nova', text: 'Titles belong inside the roster blob. Scout, find every place one leaks; Patch takes the schema.' },
      { kind: 'chip', bot: 'scout', text: 'grep · blobs.title · 2 hits' },
      { kind: 'bot', bot: 'scout', text: 'Two leaks, both on the write path: the D1 column and the chat list endpoint. Nothing else reads it.' },
      { kind: 'working', bot: 'patch', activity: 'Drafting a file' },
    ],
    hold: 1900,
  },
  {
    lines: [
      { kind: 'bot', bot: 'nova', text: 'Titles belong inside the roster blob. Scout, find every place one leaks; Patch takes the schema.' },
      { kind: 'chip', bot: 'scout', text: 'grep · blobs.title · 2 hits' },
      { kind: 'bot', bot: 'scout', text: 'Two leaks, both on the write path: the D1 column and the chat list endpoint. Nothing else reads it.' },
      { kind: 'chip', bot: 'patch', text: 'edit · schema.ts · migration 0004' },
      { kind: 'bot', bot: 'patch', text: 'Column dropped, titles move into the roster blob. Old rows keep working.' },
      { kind: 'working', bot: 'nova' },
    ],
    hold: 1300,
  },
  {
    lines: [
      { kind: 'bot', bot: 'nova', text: 'Titles belong inside the roster blob. Scout, find every place one leaks; Patch takes the schema.' },
      { kind: 'chip', bot: 'scout', text: 'grep · blobs.title · 2 hits' },
      { kind: 'bot', bot: 'scout', text: 'Two leaks, both on the write path: the D1 column and the chat list endpoint. Nothing else reads it.' },
      { kind: 'chip', bot: 'patch', text: 'edit · schema.ts · migration 0004' },
      { kind: 'bot', bot: 'patch', text: 'Column dropped, titles move into the roster blob. Old rows keep working.' },
      { kind: 'bot', bot: 'nova', text: 'Done. The relay never sees a title again.' },
    ],
    hold: 4200,
  },
]

function Avatar({ bot, size = 'size-6', working = false }: { bot: Bot; size?: string; working?: boolean }) {
  return (
    <span className={`relative inline-flex ${size} shrink-0 items-center justify-center rounded-full bg-gradient-to-b ${bot.hue} text-[10px] font-bold text-white`}>
      <span className="scale-90">{bot.glyph}</span>
      {working && (
        <span className="absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full bg-emerald-400 ring-2 ring-[#1c1c1f]">
          <span className="breathe block size-full rounded-full bg-emerald-400" />
        </span>
      )}
    </span>
  )
}

export function ChatDemo() {
  const [frame, setFrame] = useState(0)
  useEffect(() => {
    const id = setTimeout(() => setFrame((f) => (f + 1) % script.length), script[frame].hold)
    return () => clearTimeout(id)
  }, [frame])

  const lines = script[frame].lines
  const working = lines.find((l) => l.kind === 'working')
  const workingBot = working?.kind === 'working' ? working.bot : null

  return (
    <div className="window-frame overflow-hidden bg-[#1c1c1f] text-[13px] text-zinc-200">
      {/* Title bar */}
      <div className="flex items-center gap-2 border-b border-white/8 px-4 py-2.5">
        <span className="flex gap-1.5">
          <span className="size-3 rounded-full bg-[#ff5f57]" />
          <span className="size-3 rounded-full bg-[#febc2e]" />
          <span className="size-3 rounded-full bg-[#28c840]" />
        </span>
        <span className="ml-3 font-medium text-zinc-100">Ship the relay</span>
        <span className="text-zinc-500">Group · 3 bots · 2 Runners</span>
      </div>
      <div className="grid sm:grid-cols-[190px_1fr]">
        {/* Sidebar */}
        <aside className="hidden border-r border-white/8 p-3 sm:block">
          <p className="px-2 pb-2 text-[11px] font-semibold uppercase tracking-wide text-zinc-500">Chats</p>
          <div className="rounded-lg bg-white/8 px-2 py-2">
            <div className="flex items-center gap-2">
              <span className="flex -space-x-1.5">
                <Avatar bot={bots.nova} size="size-5" />
                <Avatar bot={bots.scout} size="size-5" />
                <Avatar bot={bots.patch} size="size-5" />
              </span>
              <span className="font-medium">Ship the relay</span>
            </div>
          </div>
          {(['nova', 'scout', 'patch'] as const).map((id) => (
            <div key={id} className="flex items-center gap-2 px-2 py-2 text-zinc-300">
              <Avatar bot={bots[id]} size="size-5" working={workingBot === id} />
              <span>{bots[id].name}</span>
            </div>
          ))}
          <p className="px-2 pt-4 pb-2 text-[11px] font-semibold uppercase tracking-wide text-zinc-500">Devices</p>
          {['Workbench', 'Studio'].map((d) => (
            <div key={d} className="flex items-center justify-between px-2 py-1.5 text-zinc-300">
              <span>{d}</span>
              <span className="size-1.5 rounded-full bg-emerald-400" />
            </div>
          ))}
        </aside>
        {/* Transcript */}
        <div className="flex min-h-[340px] flex-col justify-end gap-2.5 p-4">
          {lines.map((line, i) => {
            const key = `${frame}-${i}`
            if (line.kind === 'you')
              return (
                <div key={key} className="rise self-end rounded-2xl bg-[#2f7bff] px-3.5 py-2 text-white">
                  {line.text}
                </div>
              )
            if (line.kind === 'chip')
              return (
                <div key={key} className="rise ml-8 inline-flex w-fit items-center gap-2 rounded-full border border-white/10 bg-white/5 px-3 py-1 font-mono text-[11px] text-zinc-400">
                  {line.text}
                </div>
              )
            if (line.kind === 'working') {
              const bot = bots[line.bot]
              return (
                <div key={key} className="rise flex items-center gap-2.5 pt-1 text-zinc-400">
                  <span className="breathe inline-flex"><Avatar bot={bot} /></span>
                  <span>{line.activity ? `${line.activity}…` : `${bot.name} is working…`}</span>
                </div>
              )
            }
            const bot = bots[line.bot]
            return (
              <div key={key} className="rise flex max-w-[86%] flex-col">
                <span className="mb-1 ml-9 text-[11px] font-semibold text-zinc-400">{bot.name}</span>
                <div className="flex items-end gap-3">
                  <Avatar bot={bot} />
                  <div className="rounded-2xl bg-white/8 px-3.5 py-2 text-zinc-100">{line.text}</div>
                </div>
              </div>
            )
          })}
          <div className="mt-3 flex items-center gap-2 rounded-full border border-white/10 bg-white/5 px-4 py-2.5 text-zinc-500">
            <span className="text-lg leading-none">+</span>
            <span>Message Ship the relay — @ to address one bot</span>
          </div>
        </div>
      </div>
    </div>
  )
}
