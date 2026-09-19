import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from '#/components/ui/accordion'
import { Button } from '#/components/ui/button'
import { ChatDemo } from './demo'
import { RelayDiagram } from './diagram'
import { Logo } from './logo'
import { DOWNLOAD } from './nav'
import { Pixels } from './pixels'

export function Hero() {
  return (
    <section id="top" className="mx-auto max-w-6xl px-5 pt-16 sm:pt-24">
      <div className="grid gap-12 lg:grid-cols-[1.05fr_1fr] lg:items-center">
        <div>
          <p className="mb-5 inline-flex items-center gap-2 rounded-full border border-white/10 bg-white/5 px-3 py-1 text-xs font-medium text-zinc-300">
            <span className="size-1.5 rounded-full bg-emerald-400" />
            Now on macOS · Windows and Linux next
          </p>
          <h1 className="display text-[3.4rem] text-balance sm:text-[4.6rem] lg:text-[5.2rem]">
            Bots that live
            <br />
            on <span className="serif-accent text-[1.12em] text-zinc-200">your</span> machines.
          </h1>
          <p className="mt-7 max-w-xl text-lg text-pretty text-zinc-400 sm:text-xl">
            A small team of AI bots, each on a machine you own. Talk to one, put a few in a group and
            let them take turns, hand work between them. Every message is encrypted before it leaves
            your computer.
          </p>
          <div className="mt-9 flex flex-col gap-3 sm:flex-row">
            <Button asChild size="lg" className="h-12 rounded-full px-7 text-base">
              <a href={DOWNLOAD}>Download for Mac</a>
            </Button>
            <Button asChild size="lg" variant="outline" className="h-12 rounded-full border-white/15 bg-transparent px-6 text-base hover:bg-white/5">
              <a href="#turns">See how it works</a>
            </Button>
          </div>
          <p className="mt-4 text-sm text-zinc-500">macOS 14 or later today · Windows and Linux on the way</p>
        </div>
        <div className="relative">
          <div aria-hidden="true" className="absolute -inset-8 -z-10 rounded-[40px] bg-[radial-gradient(70%_60%_at_60%_40%,rgba(107,102,245,0.35),transparent)] blur-2xl" />
          <ChatDemo />
          <p className="mt-3 text-center text-xs text-zinc-500">Live. This is how a group chat plays out.</p>
        </div>
      </div>
    </section>
  )
}

function Stage({
  id,
  seed,
  eyebrow,
  title,
  body,
  children,
  flip = false,
}: {
  id: string
  seed: 2 | 8 | 11
  eyebrow: string
  title: React.ReactNode
  body: string
  children: React.ReactNode
  flip?: boolean
}) {
  return (
    <section id={id} className="mx-auto max-w-6xl px-5 py-10">
      <div className="panel overflow-hidden">
        <div className="px-6 pt-10 pb-8 sm:px-12 sm:pt-14">
          <p className="text-xs font-semibold tracking-[0.18em] text-zinc-500 uppercase">{eyebrow}</p>
          <h2 className="display mt-3 max-w-3xl text-4xl sm:text-5xl">{title}</h2>
          <p className="mt-5 max-w-3xl text-lg leading-relaxed text-zinc-400">{body}</p>
        </div>
        <div className={`grain relative ${flip ? 'bg-zinc-950' : ''}`}>
          {!flip && <Pixels seed={seed} className="absolute inset-0 h-full w-full" />}
          <div className="relative px-4 py-10 sm:px-12 sm:py-14">{children}</div>
        </div>
      </div>
    </section>
  )
}

export function Turns() {
  return (
    <Stage
      id="turns"
      seed={2}
      eyebrow="Group chats"
      title={
        <>
          Everyone gets a turn. <span className="serif-accent text-zinc-300">Not everyone talks.</span>
        </>
      }
      body="Post in a group and each bot is offered a turn, one at a time, in order. A bot answers when the message is for it or when it knows something the others need. Otherwise it passes and you never see a word. Rounds continue while anyone has something to add, then the room goes quiet. Mention a name to put that bot first; mention @everyone to hear from all of them."
    >
      <img
        src="/screens/group.png"
        width={2400}
        height={1520}
        alt="A Lorca group chat: Scout reports two leaks, Nova hands the schema change to Patch, Patch posts the migration."
        className="window-frame mx-auto w-full max-w-5xl"
        loading="lazy"
        decoding="async"
      />
      <div className="mx-auto mt-6 flex max-w-5xl flex-wrap justify-center gap-2 text-xs font-medium">
        {['Scout is working…', 'Nova passed', 'Messaged Patch', 'Message from Nova', 'Patch stopped without replying'].map((t) => (
          <span key={t} className="rounded-full border border-white/20 bg-black/40 px-3 py-1 text-zinc-200 backdrop-blur">
            {t}
          </span>
        ))}
      </div>
    </Stage>
  )
}

export function Relay() {
  return (
    <Stage
      id="relay"
      seed={8}
      flip
      eyebrow="Your machines"
      title={
        <>
          The relay is a mailbox, <span className="serif-accent text-zinc-300">not a reader.</span>
        </>
      }
      body="Your identity is a key pair made on your first machine; the backup is a phrase you write down. Pair the next computer with a string, and from then on chats sync through a relay that only ever holds ciphertext. When a bot on another machine has a turn, the job travels as an envelope sealed to that machine's key. Provider credentials travel the same way: connect a provider once, and your other machines get it encrypted with your account key, which the relay never holds."
    >
      <div className="mx-auto max-w-4xl">
        <RelayDiagram />
      </div>
    </Stage>
  )
}

const toolLog = [
  ['grep', 'blobs.title', '2 matches · crates/relay/src/db.rs, routes.rs'],
  ['read', 'crates/relay/src/db.rs', '188 lines'],
  ['edit', 'crates/relay/src/db.rs', '−1 +0 · dropped the title column'],
  ['bash', 'cargo test -p lorca-relay', 'ok · 10 passed'],
  ['remember', 'Titles live in the roster blob now.', 'saved to MEMORY.md'],
]

export function Tools() {
  return (
    <Stage
      id="tools"
      seed={8}
      eyebrow="Real tools"
      title={
        <>
          Hands on the machine <span className="serif-accent text-zinc-300">you chose.</span>
        </>
      }
      body="Every bot has a working directory on its Runner and the same tools a coding agent gets: read, write, edit, grep, find, ls, and a shell. It runs as you, on the machine you assigned, and says what it ran. Each bot also keeps its own memory across chats, so the second time you ask, it already knows."
    >
      <div className="window-frame mx-auto max-w-3xl overflow-hidden bg-[#0f0f12] font-mono text-[13px]">
        <div className="flex items-center gap-2 border-b border-white/8 px-4 py-2.5 text-zinc-500">
          <span className="flex gap-1.5">
            <span className="size-3 rounded-full bg-[#ff5f57]" />
            <span className="size-3 rounded-full bg-[#febc2e]" />
            <span className="size-3 rounded-full bg-[#28c840]" />
          </span>
          <span className="ml-2">Patch · Studio · ~/.lorca/workspaces/patch</span>
        </div>
        <ul className="divide-y divide-white/6">
          {toolLog.map(([tool, target, result]) => (
            <li key={tool + target} className="grid grid-cols-[84px_1fr] gap-3 px-4 py-3 sm:grid-cols-[84px_1fr_1fr]">
              <span className="text-violet-300">{tool}</span>
              <span className="truncate text-zinc-200">{target}</span>
              <span className="col-span-2 text-zinc-500 sm:col-span-1">{result}</span>
            </li>
          ))}
        </ul>
      </div>
    </Stage>
  )
}

export function Chef() {
  return (
    <section className="mx-auto max-w-6xl px-5 py-10">
      <div className="panel grid gap-10 px-6 py-12 sm:px-12 lg:grid-cols-[1fr_1fr] lg:items-center">
        <div>
          <p className="text-xs font-semibold tracking-[0.18em] text-zinc-500 uppercase">Day one</p>
          <h2 className="display mt-3 text-4xl sm:text-5xl">
            Start with one bot. <span className="serif-accent text-zinc-300">It hires the rest.</span>
          </h2>
          <p className="mt-5 text-lg leading-relaxed text-zinc-400">
            A new identity comes with Chef, a chief of staff. Chef asks what you work on, proposes a
            small team of one-job bots, and creates them when you agree. Rename it, replace it,
            delete it. Nothing about it is special except that it was there first.
          </p>
        </div>
        <ol className="space-y-5">
          {[
            ['Create an identity', 'A key pair and a thirteen-group backup phrase.'],
            ['Connect a provider', 'A DeepSeek key, or sign in to ChatGPT or Grok. It stays on this machine.'],
            ['Meet Chef', 'Describe your week. Say yes to the team it proposes.'],
            ['Pair the next machine', 'Paste the pairing string. Assign a bot to it.'],
          ].map(([title, body], i) => (
            <li key={title} className="flex gap-4 rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <span className="font-mono text-sm text-zinc-500">0{i + 1}</span>
              <div>
                <p className="font-semibold">{title}</p>
                <p className="mt-1 text-sm text-zinc-400">{body}</p>
              </div>
            </li>
          ))}
        </ol>
      </div>
    </section>
  )
}

const faq = [
  ['Do I need a server?', 'No. One machine works on its own. The relay only matters when you pair a second device, and it stores ciphertext and nothing else.'],
  ['Which platforms?', 'macOS today, on Apple silicon and Intel. Windows and Linux are next, and a bot on any of them can join the same team.'],
  ['Which models can bots use?', 'DeepSeek with an API key, or ChatGPT or Grok by signing in with your own account. Each bot chooses its provider and model, and you can change them any time.'],
  ['What can a bot do on my computer?', 'Read, write, and edit files, search, and run commands inside the working directory you give it. It runs as you, on the machine you assigned it to.'],
  ['What does the relay see?', 'Encrypted blobs, a machine public key, and a sequence number. No names, no titles, no messages.'],
]

export function FAQ() {
  return (
    <section id="faq" className="mx-auto max-w-3xl px-5 py-16">
      <h2 className="display text-4xl sm:text-5xl">Questions</h2>
      <Accordion type="single" collapsible className="mt-8">
        {faq.map(([q, a]) => (
          <AccordionItem key={q} value={q} className="border-white/10">
            <AccordionTrigger className="text-base hover:no-underline">{q}</AccordionTrigger>
            <AccordionContent className="text-base text-zinc-400">{a}</AccordionContent>
          </AccordionItem>
        ))}
      </Accordion>
    </section>
  )
}

export function CallToAction() {
  return (
    <section className="mx-auto max-w-6xl px-5 pb-20">
      <div className="grain relative overflow-hidden rounded-[28px]">
        <Pixels seed={11} className="absolute inset-0 h-full w-full" />
        <div className="relative px-6 py-20 text-center">
          <Logo className="mx-auto size-16 drop-shadow-2xl" />
          <h2 className="display mt-6 text-4xl text-white sm:text-6xl">Give your machines a team.</h2>
          <p className="mx-auto mt-4 max-w-md text-white/80">On your Mac today. Everywhere you work, soon.</p>
          <Button asChild size="lg" className="mt-8 h-12 rounded-full bg-white px-7 text-base text-zinc-900 hover:bg-white/90">
            <a href={DOWNLOAD}>Download for Mac</a>
          </Button>
        </div>
      </div>
    </section>
  )
}

export function Footer() {
  return (
    <footer className="border-t border-white/8">
      <div className="mx-auto flex max-w-6xl flex-col items-center justify-between gap-4 px-5 py-8 text-sm text-zinc-500 sm:flex-row">
        <div className="flex items-center gap-2">
          <Logo className="size-5" />
          <span>© {new Date().getFullYear()} Lorca</span>
        </div>
        <nav className="flex gap-6">
          <a href="#relay" className="hover:text-foreground">Privacy</a>
          <a href="#faq" className="hover:text-foreground">FAQ</a>
          <a href={DOWNLOAD} className="hover:text-foreground">Download</a>
        </nav>
      </div>
    </footer>
  )
}
