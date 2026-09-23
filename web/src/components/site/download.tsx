import { createServerFn } from '@tanstack/react-start'
import {
  Check,
  Copy,
  Laptop,
  type LucideIcon,
  Monitor,
  Smartphone,
  SquareTerminal,
  TabletSmartphone,
} from 'lucide-react'
import { Fragment, useState } from 'react'
import { Trans, useTranslation } from 'react-i18next'

import { Button } from '#/components/ui/button'
import { i18nFor, type Language, languages } from '#/i18n'
import { Logo } from './logo'
import { Nav, SITE, docsPath, downloadPath } from './nav'
import { Footer } from './sections'

/// Sparkle's feed for the Mac app. Every release rewrites it.
const APPCAST = 'https://mac-releases.lorca.app/appcast.xml'
const TESTFLIGHT = 'https://testflight.apple.com/join/WRR2R3y1'
/// The CLI's install commands, one per shell.
const INSTALL = [
  { shell: 'download.cli.unix', prompt: '$', command: 'curl -fsSL https://lorca.app/install-cli.sh | sh' },
  { shell: 'download.cli.windows', prompt: 'PS>', command: 'irm https://lorca.app/install-cli.ps1 | iex' },
] as const

export type MacRelease = {
  version: string
  /// The notarized disk image.
  url: string
  minimumSystemVersion: string | null
  appleSilicon: boolean
}

const numeric = new Intl.Collator('en', { numeric: true })

/// The newest release in a Sparkle appcast. An item's enclosure is the zip Sparkle installs
/// updates from; the release uploads the disk image people download beside it, under the same
/// name.
export function parseAppcast(xml: string): MacRelease | null {
  const items = [...xml.matchAll(/<item\b[^>]*>([\s\S]*?)<\/item>/g)].flatMap(([, item]) => {
    // Delta updates carry enclosures of their own.
    const body = item.replace(/<sparkle:deltas>[\s\S]*?<\/sparkle:deltas>/g, '')
    const field = (name: string) => body.match(new RegExp(`<${name}>([^<]*)</${name}>`))?.[1].trim()
    const archive = body.match(/<enclosure\b[^>]*\burl="([^"]+)"/)?.[1]
    const build = field('sparkle:version')
    if (!archive || !build) return []
    const release: MacRelease = {
      version: field('sparkle:shortVersionString') ?? build,
      url: archive.replace(/\.zip$/, '.dmg'),
      minimumSystemVersion: field('sparkle:minimumSystemVersion') ?? null,
      appleSilicon: field('sparkle:hardwareRequirements')?.includes('arm64') ?? false,
    }
    return [{ build, release }]
  })
  return items.sort((a, b) => numeric.compare(b.build, a.build))[0]?.release ?? null
}

/// The loader of the download page, in either language. The feed sends no CORS headers, so the
/// server reads it. A feed it can't read leaves the Mac button disabled and the rest of the page
/// working.
export const latestMacRelease = createServerFn({ method: 'GET' }).handler(async () => {
  try {
    const response = await fetch(APPCAST, { signal: AbortSignal.timeout(5000) })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const release = parseAppcast(await response.text())
    if (!release) throw new Error('no release in the feed')
    return release
  } catch (error) {
    console.error(`reading ${APPCAST}:`, error)
    return null
  }
})

/// The head for the download page in one language, paired with the other languages.
export function downloadHead(lng: Language) {
  const t = i18nFor(lng).t
  return {
    meta: [
      { title: t('download.title') },
      { name: 'description', content: t('download.description') },
      { property: 'og:title', content: t('download.title') },
      { property: 'og:description', content: t('download.description') },
    ],
    links: [
      { rel: 'canonical', href: SITE + downloadPath(lng) },
      ...languages.map((other) => ({ rel: 'alternate', hrefLang: other, href: SITE + downloadPath(other) })),
    ],
  }
}

const action = 'h-12 rounded-full px-7 text-base'

export function Download({ release }: { release: MacRelease | null }) {
  const { t, i18n } = useTranslation()
  return (
    <div className="flex min-h-svh flex-col">
      <Nav />
      <main className="flex-1">
        <section className="mx-auto max-w-4xl px-5 pt-20 text-center sm:pt-28">
          <Logo className="mx-auto size-20 drop-shadow-xl" />
          <h1 className="display mt-6 text-[3.4rem] text-balance sm:text-[4.6rem]">{t('download.title')}</h1>
        </section>
        <section className="mx-auto grid max-w-4xl gap-5 px-5 pt-12 sm:pt-16 md:grid-cols-2">
          <Platform
            icon={Laptop}
            title={t('download.mac.title')}
            body={t('download.mac.body')}
            note={release ? <Requirements release={release} /> : t('download.mac.unavailable')}
          >
            {release ? (
              <Button asChild size="lg" className={action}>
                <a href={release.url}>{t('download.mac.action')}</a>
              </Button>
            ) : (
              <Button size="lg" disabled className={action}>
                {t('download.mac.action')}
              </Button>
            )}
          </Platform>
          <Platform icon={TabletSmartphone} title={t('download.ios.title')} body={t('download.ios.body')} note={t('download.ios.note')}>
            <Button
              asChild
              size="lg"
              variant="outline"
              className={`${action} bg-transparent shadow-none hover:bg-foreground/5 dark:bg-transparent dark:hover:bg-foreground/5`}
            >
              <a href={TESTFLIGHT}>{t('download.ios.action')}</a>
            </Button>
          </Platform>
          <Platform icon={Monitor} title={t('download.windowsLinux.title')} body={t('download.windowsLinux.body')}>
            <Soon />
          </Platform>
          <Platform icon={Smartphone} title={t('download.android.title')} body={t('download.android.body')}>
            <Soon />
          </Platform>
        </section>
        <section className="mx-auto max-w-4xl px-5 pt-5 pb-20">
          <Platform
            icon={SquareTerminal}
            title={t('download.cli.title')}
            body={t('download.cli.body')}
            note={
              <Trans
                i18nKey="download.cli.note"
                components={{
                  docs: <a href={`${docsPath(i18n.language)}/cli`} className="underline underline-offset-4 hover:text-foreground" />,
                }}
              />
            }
          >
            <div className="space-y-5">
              {INSTALL.map(({ shell, prompt, command }) => (
                <div key={command}>
                  <p className="mb-2 text-sm text-muted-foreground">{t(shell)}</p>
                  <InstallCommand prompt={prompt} command={command} />
                </div>
              ))}
            </div>
          </Platform>
        </section>
      </main>
      <Footer />
    </div>
  )
}

/// Version and system requirements. A narrow card wraps the line between items, never inside one.
function Requirements({ release }: { release: MacRelease }) {
  const { t } = useTranslation()
  const items = [
    t('download.mac.version', { version: release.version }),
    release.minimumSystemVersion &&
      t('download.mac.system', { version: release.minimumSystemVersion.replace(/(\.0)+$/, '') }),
    release.appleSilicon && t('download.mac.appleSilicon'),
  ].filter((item): item is string => Boolean(item))
  return items.map((item, i) => (
    <Fragment key={item}>
      {i > 0 && ' · '}
      <span className="whitespace-nowrap">{item}</span>
    </Fragment>
  ))
}

/// An install command. A click anywhere on it copies it; the text stays selectable.
function InstallCommand({ prompt, command }: { prompt: string; command: string }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  const copy = () =>
    navigator.clipboard.writeText(command).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  const Icon = copied ? Check : Copy
  return (
    <div
      onClick={copy}
      className="flex cursor-pointer items-center gap-3 rounded-2xl border bg-foreground/[0.03] px-4 py-3.5 font-mono text-sm transition-colors hover:bg-foreground/5"
    >
      <span className="text-muted-foreground/60 select-none" aria-hidden="true">
        {prompt}
      </span>
      {/* One line, like a terminal: a narrow screen scrolls it rather than wrapping the URL. */}
      <code className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap">{command}</code>
      {/* For the keyboard: its click bubbles to the block. */}
      <button
        type="button"
        aria-label={t(copied ? 'download.cli.copied' : 'download.cli.copy')}
        title={t(copied ? 'download.cli.copied' : 'download.cli.copy')}
        className="-m-1 rounded-md p-1 text-muted-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
      >
        <Icon className="size-4" />
      </button>
    </div>
  )
}

/// In place of the button, for a platform with no release yet.
function Soon() {
  const { t } = useTranslation()
  return (
    <p className="inline-flex rounded-full border bg-foreground/5 px-3 py-1 text-xs font-medium text-foreground/80">
      {t('download.soon')}
    </p>
  )
}

function Platform({
  icon: Icon,
  title,
  body,
  note,
  children,
}: {
  icon: LucideIcon
  title: string
  body: string
  /// A line under the button: version and requirements.
  note?: React.ReactNode
  /// The button, or the coming-soon mark.
  children: React.ReactNode
}) {
  return (
    <div className="panel flex flex-col px-7 py-9 sm:px-10 sm:py-10">
      <Icon className="size-6 text-violet" strokeWidth={1.75} />
      <h2 className="mt-6 text-2xl font-semibold tracking-tight">{title}</h2>
      <p className="mt-3 max-w-2xl leading-relaxed text-muted-foreground">{body}</p>
      <div className="mt-auto pt-8">
        {children}
        {note && <p className="mt-4 text-sm text-muted-foreground/80">{note}</p>}
      </div>
    </div>
  )
}
