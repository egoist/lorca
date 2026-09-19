import { Brain, FilePen, Globe, Plug, Search, SquareTerminal } from 'lucide-react'
import { Trans, useTranslation } from 'react-i18next'

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from '#/components/ui/accordion'
import { Button } from '#/components/ui/button'
import { RelayDiagram } from './diagram'
import { Logo } from './logo'
import { DOWNLOAD, LanguageLink } from './nav'
import { Pixels } from './pixels'

export function Hero() {
  const { t } = useTranslation()
  return (
    <section id="top" className="mx-auto max-w-4xl px-5 pt-20 text-center sm:pt-28">
      <p className="mb-5 inline-flex items-center gap-2 rounded-full border bg-foreground/5 px-3 py-1 text-xs font-medium text-foreground/80">
        <span className="size-1.5 rounded-full bg-emerald-500 dark:bg-emerald-400" />
        {t('hero.badge')}
      </p>
      <h1 className="display text-[3.4rem] text-balance sm:text-[4.6rem] lg:text-[5.2rem]">
        <Trans i18nKey="hero.title" components={{ accent: <span className="serif-accent text-[1.12em] text-foreground/85" /> }} />
      </h1>
      <p className="mx-auto mt-7 max-w-2xl text-lg text-pretty text-muted-foreground sm:text-xl">
        {t('hero.body')}
      </p>
      <div className="mt-9 flex flex-col justify-center gap-3 sm:flex-row">
        <Button asChild size="lg" className="h-12 rounded-full px-7 text-base">
          <a href={DOWNLOAD}>{t('nav.download')}</a>
        </Button>
        <Button asChild size="lg" variant="outline" className="h-12 rounded-full bg-transparent px-6 text-base shadow-none hover:bg-foreground/5 dark:bg-transparent dark:hover:bg-foreground/5">
          <a href="#turns">{t('hero.how')}</a>
        </Button>
      </div>
      <p className="mt-4 text-sm text-muted-foreground/80">{t('hero.platforms')}</p>
    </section>
  )
}

function Accent({ children }: { children?: React.ReactNode }) {
  return <span className="serif-accent text-foreground/80">{children}</span>
}

function Stage({
  id,
  seed,
  eyebrow,
  title,
  body,
  children,
  flip = false,
  bare = false,
}: {
  id: string
  seed: 2 | 8 | 11
  eyebrow: string
  title: React.ReactNode
  body: string
  children: React.ReactNode
  flip?: boolean
  /// No backdrop: the children sit on the panel's own surface, edge to edge.
  bare?: boolean
}) {
  return (
    <section id={id} className="mx-auto max-w-6xl px-5 py-10">
      <div className="panel overflow-hidden">
        <div className="px-6 pt-10 pb-8 sm:px-12 sm:pt-14">
          <p className="text-xs font-semibold tracking-[0.18em] text-muted-foreground/80 uppercase">{eyebrow}</p>
          <h2 className="display mt-3 max-w-3xl text-4xl sm:text-5xl">{title}</h2>
          <p className="mt-5 max-w-3xl text-lg leading-relaxed text-muted-foreground">{body}</p>
        </div>
        {bare ? (
          children
        ) : (
          <div className={`grain relative ${flip ? 'bg-zinc-950' : ''}`}>
            {!flip && <Pixels seed={seed} className="absolute inset-0 h-full w-full" />}
            <div className="relative px-4 py-10 sm:px-12 sm:py-14">{children}</div>
          </div>
        )}
      </div>
    </section>
  )
}

export function Turns() {
  const { t } = useTranslation()
  return (
    <Stage
      id="turns"
      seed={2}
      eyebrow={t('turns.eyebrow')}
      title={<Trans i18nKey="turns.title" components={{ accent: <Accent /> }} />}
      body={t('turns.body')}
    >
      <img
        src="/screens/group.png"
        width={2400}
        height={1520}
        alt={t('turns.alt')}
        className="window-frame mx-auto w-full max-w-5xl"
        loading="lazy"
        decoding="async"
      />
    </Stage>
  )
}

export function Relay() {
  const { t } = useTranslation()
  return (
    <Stage
      id="relay"
      seed={8}
      flip
      eyebrow={t('relay.eyebrow')}
      title={<Trans i18nKey="relay.title" components={{ accent: <Accent /> }} />}
      body={t('relay.body')}
    >
      <div className="mx-auto max-w-4xl">
        <RelayDiagram />
      </div>
    </Stage>
  )
}

/// What a bot can reach for on its Runner. The tool names are the ones the model calls.
const toolKinds = [
  { key: 'files', icon: FilePen, names: 'read · write · edit' },
  { key: 'search', icon: Search, names: 'grep · find · ls' },
  { key: 'shell', icon: SquareTerminal, names: 'bash' },
  { key: 'web', icon: Globe, names: 'web_search · web_fetch' },
  { key: 'memory', icon: Brain, names: 'MEMORY.md' },
  { key: 'plugins', icon: Plug, names: 'MCP' },
] as const

export function Tools() {
  const { t } = useTranslation()
  return (
    <Stage
      id="tools"
      seed={8}
      bare
      eyebrow={t('tools.eyebrow')}
      title={<Trans i18nKey="tools.title" components={{ accent: <Accent /> }} />}
      body={t('tools.body')}
    >
      {/* Hairlines are the grid's own background showing through one-pixel gaps. */}
      <ul className="grid gap-px bg-border pt-px sm:grid-cols-2 lg:grid-cols-3">
        {toolKinds.map(({ key, icon: Icon, names }) => (
          <li key={key} className="bg-card px-6 py-8 sm:px-12">
            <Icon className="size-5 text-violet" strokeWidth={1.75} />
            <p className="mt-5 flex flex-wrap items-baseline gap-x-3 gap-y-1">
              <span className="font-semibold">{t(`tools.kinds.${key}.title`)}</span>
              <span className="font-mono text-xs text-muted-foreground/80">{names}</span>
            </p>
            <p className="mt-2 leading-relaxed text-muted-foreground">{t(`tools.kinds.${key}.body`)}</p>
          </li>
        ))}
      </ul>
    </Stage>
  )
}

export function Chef() {
  const { t } = useTranslation()
  return (
    <section className="mx-auto max-w-6xl px-5 py-10">
      <div className="panel grid gap-10 px-6 py-12 sm:px-12 lg:grid-cols-[1fr_1fr] lg:items-center">
        <div>
          <p className="text-xs font-semibold tracking-[0.18em] text-muted-foreground/80 uppercase">{t('chef.eyebrow')}</p>
          <h2 className="display mt-3 text-4xl sm:text-5xl">
            <Trans i18nKey="chef.title" components={{ accent: <Accent /> }} />
          </h2>
          <p className="mt-5 text-lg leading-relaxed text-muted-foreground">
            {t('chef.body')}
          </p>
        </div>
        <ol className="space-y-5">
          {t('chef.steps', { returnObjects: true }).map(({ title, body }, i) => (
            <li key={title} className="flex gap-4 rounded-2xl border bg-foreground/[0.03] p-4">
              <span className="font-mono text-sm text-muted-foreground/80">0{i + 1}</span>
              <div>
                <p className="font-semibold">{title}</p>
                <p className="mt-1 text-sm text-muted-foreground">{body}</p>
              </div>
            </li>
          ))}
        </ol>
      </div>
    </section>
  )
}

export function FAQ() {
  const { t } = useTranslation()
  return (
    <section id="faq" className="mx-auto max-w-3xl px-5 py-16">
      <h2 className="display text-4xl sm:text-5xl">{t('faq.title')}</h2>
      <Accordion type="single" collapsible className="mt-8">
        {t('faq.items', { returnObjects: true }).map(({ q, a }) => (
          <AccordionItem key={q} value={q}>
            <AccordionTrigger className="text-base hover:no-underline">{q}</AccordionTrigger>
            <AccordionContent className="text-base text-muted-foreground">{a}</AccordionContent>
          </AccordionItem>
        ))}
      </Accordion>
    </section>
  )
}

export function CallToAction() {
  const { t } = useTranslation()
  return (
    <section className="mx-auto max-w-6xl px-5 pb-20">
      <div className="grain relative overflow-hidden rounded-[28px]">
        <Pixels seed={11} className="absolute inset-0 h-full w-full" />
        <div className="relative px-6 py-20 text-center">
          <Logo className="mx-auto size-16 drop-shadow-2xl" />
          <h2 className="display mt-6 text-4xl text-white sm:text-6xl">{t('cta.title')}</h2>
          <p className="mx-auto mt-4 max-w-md text-white/80">{t('cta.body')}</p>
          <Button asChild size="lg" className="mt-8 h-12 rounded-full bg-white px-7 text-base text-zinc-900 hover:bg-white/90">
            <a href={DOWNLOAD}>{t('nav.download')}</a>
          </Button>
        </div>
      </div>
    </section>
  )
}

export function Footer() {
  const { t } = useTranslation()
  return (
    <footer className="border-t">
      <div className="mx-auto flex max-w-6xl flex-col items-center justify-between gap-4 px-5 py-8 text-sm text-muted-foreground/80 sm:flex-row">
        <div className="flex items-center gap-2">
          <Logo className="size-5" />
          <span>© {new Date().getFullYear()} Lorca</span>
        </div>
        <nav className="flex gap-6">
          <a href="#relay" className="hover:text-foreground">{t('footer.privacy')}</a>
          <a href="#faq" className="hover:text-foreground">{t('footer.faq')}</a>
          <a href={DOWNLOAD} className="hover:text-foreground">{t('footer.download')}</a>
          <LanguageLink className="hover:text-foreground" />
        </nav>
      </div>
    </footer>
  )
}
