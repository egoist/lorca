import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '#/components/ui/button'
import { type Language, languages, names, paths } from '#/i18n'
import { Logo } from './logo'

/// Where the macOS app installer lives. Other desktop computers run the CLI directly.
export const DOWNLOAD = '/download'
export const SITE = 'https://lorca.app'

const links = [
  { href: '#turns', label: 'nav.turns' },
  { href: '#relay', label: 'nav.relay' },
  { href: '#tools', label: 'nav.tools' },
  { href: '#faq', label: 'nav.faq' },
] as const

/// The docs in the page's language: `/docs` and `/zh/docs`.
export function docsPath(lng: string) {
  return lng === 'en' ? '/docs' : `/${lng}/docs`
}

/// A link to the same page in the other language.
export function LanguageLink({ className }: { className?: string }) {
  const { i18n } = useTranslation()
  const other = languages.find((lng) => lng !== i18n.language) as Language
  return (
    <Link to={paths[other]} lang={other} className={className}>
      {names[other]}
    </Link>
  )
}

export function Nav() {
  const { t, i18n } = useTranslation()
  return (
    <header className="sticky top-0 z-40 border-b bg-background/70 backdrop-blur-xl">
      <div className="mx-auto flex h-14 max-w-6xl items-center justify-between px-5">
        <a href="#top" className="flex items-center gap-2.5 font-semibold tracking-tight">
          <Logo className="size-7" />
          Lorca
        </a>
        <nav className="hidden items-center gap-7 text-sm text-muted-foreground md:flex">
          {links.map((link) => (
            <a key={link.href} href={link.href} className="transition-colors hover:text-foreground">
              {t(link.label)}
            </a>
          ))}
          <a href={docsPath(i18n.language)} className="transition-colors hover:text-foreground">
            {t('nav.docs')}
          </a>
        </nav>
        <div className="flex items-center gap-4">
          <LanguageLink className="text-sm text-muted-foreground transition-colors hover:text-foreground" />
          <Button asChild size="sm" className="rounded-full px-4">
            <a href={DOWNLOAD}>{t('nav.download')}</a>
          </Button>
        </div>
      </div>
    </header>
  )
}
