import { Link } from '@tanstack/react-router'
import { xchacha20poly1305 } from '@noble/ciphers/chacha.js'
import {
  Activity,
  Binoculars,
  Book,
  Brain,
  Calendar,
  ChartColumnBig,
  ClipboardList,
  CodeXml,
  Eye,
  FileText,
  Flame,
  FlaskConical,
  Globe,
  Hammer,
  Inbox,
  Leaf,
  ListChecks,
  Lock,
  type LucideIcon,
  Mail,
  Paintbrush,
  Search,
  Server,
  Shield,
  Signature,
  Sparkles,
  SquareTerminal,
  Sun,
  TrendingUp,
  TriangleAlert,
  User,
  WandSparkles,
  Zap,
} from 'lucide-react'
import { useEffect, useState } from 'react'
import { I18nextProvider, Trans, useTranslation } from 'react-i18next'

import { Button } from '#/components/ui/button'
import { i18nFor, type Language, names, paths } from '#/i18n'
import { Logo } from './logo'
import { downloadPath } from './nav'

/// A shared bot template: https://lorca.app/t/<id>#<key>, with `?relay=<origin>` when it sits on
/// a relay other than Lorca's. The relay holds the template sealed with XChaCha20-Poly1305 under
/// the key in the fragment, which a browser never sends; this page reads the ciphertext, opens it
/// here, and shows what the bot would bring. Nothing about the bot leaves the browser.

const DEFAULT_RELAY = 'https://relay.lorca.app'
/// Associated data of a shared template's envelope, as the CLI seals it.
const ENVELOPE_KIND = 'template'
const NONCE_BYTES = 24

type Profile = { name: string; description?: string; symbol_name?: string; accent?: string }
export type Template = {
  format: string
  version: number
  profile?: Profile
  skills?: { name: string; description?: string }[]
  memories?: string[]
  routines?: { name: string; schedule?: string; prompt?: string }[]
  requirements?: { service_id: string }[]
}

type Problem = 'noKey' | 'notFound' | 'badKey' | 'notTemplate' | 'unreachable'
type State = { kind: 'loading' } | { kind: 'problem'; problem: Problem } | { kind: 'ready'; template: Template }

/// Unpadded base64url to bytes; `null` for anything else.
export function fromBase64url(text: string): Uint8Array | null {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) return null
  try {
    const binary = atob(text.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (text.length % 4)) % 4))
    return Uint8Array.from(binary, (c) => c.charCodeAt(0))
  } catch {
    return null
  }
}

/// Opens a shared template's envelope (`nonce || ciphertext`) with its key. Throws when the key
/// doesn't open it.
export function openTemplate(envelope: Uint8Array, key: Uint8Array): unknown {
  const nonce = envelope.subarray(0, NONCE_BYTES)
  const cipher = xchacha20poly1305(key, nonce, new TextEncoder().encode(ENVELOPE_KIND))
  return JSON.parse(new TextDecoder().decode(cipher.decrypt(envelope.subarray(NONCE_BYTES))))
}

function isTemplate(value: unknown): value is Template {
  const template = value as Template
  return typeof template === 'object' && template !== null && template.format === 'lorca.bot-template' && template.version === 1
}

/// The relay a link names, or Lorca's.
function relayOf(search: string): string | null {
  const named = new URLSearchParams(search).get('relay')
  if (!named) return DEFAULT_RELAY
  try {
    const url = new URL(named)
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.origin + url.pathname.replace(/\/+$/, '') : null
  } catch {
    return null
  }
}

async function load(id: string, search: string, fragment: string): Promise<State> {
  const key = fromBase64url(fragment)
  if (!key || key.length !== 32) return { kind: 'problem', problem: 'noKey' }
  const relay = relayOf(search)
  if (!relay || !/^[A-Za-z0-9._-]{1,64}$/.test(id)) return { kind: 'problem', problem: 'notFound' }
  let response: Response
  try {
    response = await fetch(`${relay}/v1/shares/${id}`, { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' })
  } catch {
    return { kind: 'problem', problem: 'unreachable' }
  }
  if (response.status === 404) return { kind: 'problem', problem: 'notFound' }
  if (!response.ok) return { kind: 'problem', problem: 'unreachable' }
  let value: unknown
  try {
    value = openTemplate(new Uint8Array(await response.arrayBuffer()), key)
  } catch {
    return { kind: 'problem', problem: 'badKey' }
  }
  return isTemplate(value) ? { kind: 'ready', template: value } : { kind: 'problem', problem: 'notTemplate' }
}

/// The app's address for the same link: `lorca://t/<id>?relay=…#<key>`.
function appLink(id: string, search: string, fragment: string): string {
  const relay = new URLSearchParams(search).get('relay')
  return `lorca://t/${id}${relay ? `?relay=${encodeURIComponent(relay)}` : ''}#${fragment}`
}

// MARK: - The bot's look

/// The SF Symbols a bot's look offers, as the Windows and Linux app draws them (Lucide).
const SYMBOLS: Record<string, LucideIcon> = {
  sparkles: Sparkles,
  'wand.and.stars': WandSparkles,
  'hammer.fill': Hammer,
  'book.fill': Book,
  'paintbrush.fill': Paintbrush,
  'chart.bar.fill': ChartColumnBig,
  'terminal.fill': SquareTerminal,
  globe: Globe,
  'brain.head.profile': Brain,
  magnifyingglass: Search,
  'envelope.fill': Mail,
  calendar: Calendar,
  'flask.fill': FlaskConical,
  'bolt.fill': Zap,
  'leaf.fill': Leaf,
  'shield.fill': Shield,
  'binoculars.fill': Binoculars,
  'chevron.left.forwardslash.chevron.right': CodeXml,
  'pencil.and.scribble': Signature,
  'bolt.horizontal.fill': Activity,
  'flame.fill': Flame,
  'list.bullet.clipboard.fill': ClipboardList,
  'server.rack': Server,
  checklist: ListChecks,
  'doc.text.fill': FileText,
  'tray.full.fill': Inbox,
  'sun.max.fill': Sun,
  'eye.fill': Eye,
  'exclamationmark.triangle.fill': TriangleAlert,
  'chart.line.uptrend.xyaxis': TrendingUp,
  'person.fill': User,
}

/// The apps' accent colors, as Apple's system colors.
const ACCENTS: Record<string, string> = {
  indigo: '#5856d6',
  blue: '#0a84ff',
  teal: '#30b0c7',
  green: '#34c759',
  orange: '#ff9500',
  pink: '#ff2d55',
  purple: '#af52de',
  red: '#ff3b30',
}

function Avatar({ profile }: { profile?: Profile }) {
  const Icon = SYMBOLS[profile?.symbol_name ?? ''] ?? Sparkles
  const color = ACCENTS[profile?.accent ?? ''] ?? ACCENTS.indigo
  return (
    <div
      className="flex size-20 items-center justify-center rounded-full text-white shadow-lg"
      style={{ backgroundImage: `linear-gradient(to bottom, color-mix(in srgb, ${color} 72%, white), ${color})` }}
      aria-hidden="true"
    >
      <Icon className="size-9" strokeWidth={2} />
    </div>
  )
}

// MARK: - What's included

/// Plugins whose names a title-cased id gets wrong, from the marketplace index.
const PLUGIN_NAMES: Record<string, string> = {
  github: 'GitHub',
  playwright: 'Browser',
  clickup: 'ClickUp',
  feishu: '飞书',
  'feishu-project': '飞书项目',
  dida365: '滴答清单',
  'tencent-docs': '腾讯文档',
  kling: '可灵',
  gitlab: 'GitLab',
  posthog: 'PostHog',
  apollo: 'Apollo.io',
  paypal: 'PayPal',
  deepwiki: 'DeepWiki',
  huggingface: 'Hugging Face',
  metaso: '秘塔 AI 搜索',
  zhihu: '知乎',
  amap: '高德地图',
}

function pluginName(id: string): string {
  return PLUGIN_NAMES[id] ?? id.split('-').map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(' ')
}

/// A memory or a prompt as one line: its first, without Markdown's list or heading marks.
function firstLine(text: string): string {
  return (
    text
      .split('\n')
      .map((line) => line.trim().replace(/^(#+|[-*+]|\d+\.)\s+/, ''))
      .find(Boolean) ?? ''
  )
}

const SHOWN_MEMORIES = 5

function Included({ template }: { template: Template }) {
  const { t } = useTranslation()
  const routines = template.routines ?? []
  const plugins = template.requirements ?? []
  const memories = template.memories ?? []
  const skills = template.skills ?? []
  if (!routines.length && !plugins.length && !memories.length && !skills.length) return null
  return (
    <section className="panel mt-5 px-7 py-8 sm:px-10">
      <h2 className="text-xl font-semibold tracking-tight">{t('template.included')}</h2>
      {routines.length > 0 && (
        <Part title={t('template.routines')} note={t('template.routinesNote')}>
          {routines.map((routine) => (
            <Row key={routine.name} title={routine.name} detail={firstLine(routine.prompt ?? '')} />
          ))}
        </Part>
      )}
      {plugins.length > 0 && (
        <Part title={t('template.plugins')} note={t('template.pluginsNote')}>
          <li className="flex flex-wrap gap-2 pt-1">
            {plugins.map((plugin) => (
              <span key={plugin.service_id} className="rounded-full border bg-foreground/[0.03] px-3 py-1 text-sm">
                {pluginName(plugin.service_id)}
              </span>
            ))}
          </li>
        </Part>
      )}
      {skills.length > 0 && (
        <Part title={t('template.skills')}>
          {skills.map((skill) => (
            <Row key={skill.name} title={skill.name} detail={skill.description ?? ''} />
          ))}
        </Part>
      )}
      {memories.length > 0 && (
        <Part title={t('template.memories')} tight>
          {memories.slice(0, SHOWN_MEMORIES).map((memory, i) => (
            <li key={i} className="leading-relaxed text-foreground/90">
              {firstLine(memory)}
            </li>
          ))}
          {memories.length > SHOWN_MEMORIES && (
            <li className="text-sm text-muted-foreground">{t('template.more', { count: memories.length - SHOWN_MEMORIES })}</li>
          )}
        </Part>
      )}
    </section>
  )
}

function Part({ title, note, tight, children }: { title: string; note?: string; tight?: boolean; children: React.ReactNode }) {
  return (
    <div className="mt-7">
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h3>
      <ul className={tight ? 'mt-3 space-y-1.5' : 'mt-3 space-y-3'}>{children}</ul>
      {note && <p className="mt-3 text-sm text-muted-foreground">{note}</p>}
    </div>
  )
}

function Row({ title, detail }: { title: string; detail: string }) {
  return (
    <li>
      <p className="font-medium">{title}</p>
      {detail && <p className="mt-0.5 line-clamp-2 text-sm leading-relaxed text-muted-foreground">{detail}</p>}
    </li>
  )
}

// MARK: - The page

const action = 'h-12 rounded-full px-7 text-base'
const link = 'underline underline-offset-4 hover:text-foreground'

/// The page for one shared bot, in the browser's language: the link is the same for everyone, so
/// the path can't pick it.
export function SharedTemplate({ id }: { id: string }) {
  const [lng, setLng] = useState<Language | null>(null)
  const [state, setState] = useState<State>({ kind: 'loading' })
  const [openLink, setOpenLink] = useState<string | null>(null)

  useEffect(() => {
    const preferred = (navigator.languages?.[0] ?? navigator.language ?? '').toLowerCase()
    setLng(preferred.startsWith('zh') ? 'zh' : 'en')
    const { search, hash } = window.location
    const fragment = hash.replace(/^#/, '')
    setOpenLink(appLink(id, search, fragment))
    let current = true
    load(id, search, fragment).then((next) => current && setState(next))
    return () => {
      current = false
    }
  }, [id])

  useEffect(() => {
    if (lng) document.documentElement.lang = lng === 'zh' ? 'zh-Hans' : 'en'
    if (state.kind === 'ready' && state.template.profile?.name) document.title = `${state.template.profile.name} · Lorca`
  }, [lng, state])

  return (
    <I18nextProvider i18n={i18nFor(lng ?? 'en')}>
      <div className="flex min-h-svh flex-col">
        <Header language={lng} onLanguage={setLng} />
        {/* As wide as the header's pill. */}
        <main className="flex-1 px-4 pt-14 pb-20 sm:pt-20">
          <div className="mx-auto max-w-2xl">
            {/* Nothing in a language until the browser says which. */}
            {lng && state.kind === 'ready' && <Bot template={state.template} appLink={openLink} />}
            {lng && state.kind === 'problem' && <Trouble problem={state.problem} />}
            {(!lng || state.kind === 'loading') && <div className="panel h-72 animate-pulse bg-foreground/[0.03]" aria-busy="true" />}
          </div>
        </main>
      </div>
    </I18nextProvider>
  )
}

function Header({ language, onLanguage }: { language: Language | null; onLanguage: (lng: Language) => void }) {
  const other: Language = language === 'zh' ? 'en' : 'zh'
  return (
    <header className="sticky top-4 z-40 mt-4 px-4">
      <div className="mx-auto flex h-14 max-w-2xl items-center justify-between rounded-full pr-5 pl-5 bg-zinc-200/50 backdrop-blur-xl dark:bg-zinc-900/80 dark:border">
        <Link to={paths[language ?? 'en']} className="flex items-center gap-2 font-semibold tracking-tight">
          <Logo className="size-10" />
          Lorca
        </Link>
        {/* Switches in place: the key in the address must stay where it is. */}
        {language && (
          <button
            type="button"
            lang={other}
            onClick={() => onLanguage(other)}
            className="text-sm text-muted-foreground transition-colors hover:text-foreground"
          >
            {names[other]}
          </button>
        )}
      </div>
    </header>
  )
}

function Bot({ template, appLink }: { template: Template; appLink: string | null }) {
  const { t, i18n } = useTranslation()
  const profile = template.profile
  return (
    <>
      <section className="panel px-7 py-9 sm:px-10 sm:py-10">
        <Avatar profile={profile} />
        {/* A name in any language: the site's Chinese headline sizes would blow up a Latin one. */}
        <h1 className="mt-6 text-4xl leading-[1.05] font-semibold tracking-[-0.035em] text-balance sm:text-5xl">
          {profile?.name ?? t('template.title')}
        </h1>
        {profile?.description && (
          <p className="mt-4 leading-relaxed whitespace-pre-line text-muted-foreground">{profile.description}</p>
        )}
        <div className="mt-8">
          <Button asChild size="lg" className={action}>
            <a href={appLink ?? undefined}>{t('template.open')}</a>
          </Button>
        </div>
        <p className="mt-5 text-sm leading-relaxed text-muted-foreground">
          <Trans i18nKey="template.noApp" components={{ download: <Link to={downloadPath(i18n.language)} className={link} /> }} />
        </p>
      </section>
      <Included template={template} />
      <p className="mt-6 flex items-center justify-center gap-2 text-center text-sm text-muted-foreground/80">
        <Lock className="size-3.5 shrink-0" />
        {t('template.privacy')}
      </p>
    </>
  )
}

function Trouble({ problem }: { problem: Problem }) {
  const { t, i18n } = useTranslation()
  return (
    <section className="panel px-7 py-10 text-center sm:px-10">
      <h1 className="text-2xl font-semibold tracking-tight">{t(`template.error.${problem}.title`)}</h1>
      <p className="mt-3 leading-relaxed text-muted-foreground">{t(`template.error.${problem}.body`)}</p>
      <Button asChild size="lg" variant="outline" className={`${action} mt-8 bg-transparent shadow-none dark:bg-transparent`}>
        <Link to={paths[i18n.language as Language]}>{t('template.home')}</Link>
      </Button>
    </section>
  )
}
