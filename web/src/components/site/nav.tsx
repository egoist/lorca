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

/// The docs in the page's language: `/docs` and `/zh/docs`.
export function docsPath(lng: string) {
  return lng === 'en' ? '/docs' : `/${lng}/docs`
}

/// The download page in the page's language: `/download` and `/zh/download`. It links the Mac
/// app's disk image, the iPhone beta, and the CLI installer.
export function downloadPath(lng: string) {
  return lng === 'en' ? '/download' : `/${lng}/download`
}

/// A section of the landing page, from any page: `/#faq` and `/zh#faq`.
export function homeSection(lng: string, id: string) {
  return `${paths[lng as Language]}#${id}`
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
        <a href={homeSection(i18n.language, 'top')} className="flex items-center gap-2 font-semibold tracking-tight">
          <Logo className="size-10" />
          Lorca
        </a>
        <nav className="hidden items-center gap-6 text-sm text-muted-foreground md:flex">
          {links.map((link) => (
            <a key={link.id} href={homeSection(i18n.language, link.id)} className="transition-colors hover:text-foreground">
              {t(link.label)}
            </a>
          ))}
          <a href={docsPath(i18n.language)} className="transition-colors hover:text-foreground">
            {t('nav.docs')}
          </a>
        </nav>
        <div className="flex items-center gap-3">
          <LanguageLink className="text-sm text-muted-foreground transition-colors hover:text-foreground" />
          <Button asChild size="sm" className="rounded-full px-4">
            <a href={downloadPath(i18n.language)}>{t('nav.download')}</a>
          </Button>
        </div>
      </div>
    </header>
  )
}
