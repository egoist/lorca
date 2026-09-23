import { Link, useLocation } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '#/components/ui/button'
import { type Language, languages, names, pathIn, paths } from '#/i18n'
import { Logo } from './logo'

export const SITE = 'https://lorca.app'

const links = [
  { id: 'turns', label: 'nav.turns' },
  { id: 'relay', label: 'nav.relay' },
  { id: 'tools', label: 'nav.tools' },
  { id: 'faq', label: 'nav.faq' },
] as const

/// The docs in the page's language: `/docs` and `/zh/docs`, or one of their pages: `/docs/cli`.
export function docsPath(lng: string, page?: string) {
  const docs = lng === 'en' ? '/docs' : `/${lng}/docs`
  return page ? `${docs}/${page}` : docs
}

/// The download page in the page's language: `/download` and `/zh/download`. It links the Mac
/// app's disk image, the iPhone beta, and the CLI installer.
export function downloadPath(lng: string) {
  return lng === 'en' ? '/download' : `/${lng}/download`
}

/// A section of the landing page, from any page: `/#faq` and `/zh#faq`. It is the current page
/// only at its own section. Without `resetScroll`, a second click on the section already in the
/// address bar restores the scroll position the click was made at instead of scrolling to it.
export function SectionLink({ id, ...props }: { id: string } & Omit<React.ComponentProps<'a'>, 'href'>) {
  const { i18n } = useTranslation()
  return (
    <Link
      to={paths[i18n.language as Language]}
      hash={id}
      activeOptions={{ includeHash: true }}
      resetScroll={false}
      {...props}
    />
  )
}

/// A link to the same page in the other language.
export function LanguageLink({ className }: { className?: string }) {
  const { i18n } = useTranslation()
  const pathname = useLocation({ select: (location) => location.pathname })
  const other = languages.find((lng) => lng !== i18n.language) as Language
  return (
    <Link to={pathIn(other, pathname)} lang={other} className={className}>
      {names[other]}
    </Link>
  )
}

/// A floating pill, clear of the top edge, that stays put as the page scrolls.
export function Nav() {
  const { t, i18n } = useTranslation()
  return (
    <header className="sticky top-4 z-40 mt-4 px-4">
      <div className="mx-auto flex h-14 max-w-4xl items-center justify-between rounded-full pr-4 pl-5 bg-zinc-200/50 backdrop-blur-xl dark:bg-zinc-900/80 dark:border">
        <SectionLink id="top" className="flex items-center gap-2 font-semibold tracking-tight">
          <Logo className="size-10" />
          Lorca
        </SectionLink>
        <nav className="hidden items-center gap-6 text-sm text-muted-foreground md:flex">
          {links.map((link) => (
            <SectionLink key={link.id} id={link.id} className="transition-colors hover:text-foreground">
              {t(link.label)}
            </SectionLink>
          ))}
          <Link to={docsPath(i18n.language)} className="transition-colors hover:text-foreground">
            {t('nav.docs')}
          </Link>
        </nav>
        <div className="flex items-center gap-3">
          <LanguageLink className="text-sm text-muted-foreground transition-colors hover:text-foreground" />
          <Button asChild size="sm" className="rounded-full px-4">
            <Link to={downloadPath(i18n.language)}>{t('nav.download')}</Link>
          </Button>
        </div>
      </div>
    </header>
  )
}
