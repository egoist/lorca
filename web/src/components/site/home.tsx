import { i18nFor, type Language, languages, paths } from '#/i18n'
import { Nav, SITE } from './nav'
import { CallToAction, Chef, FAQ, Footer, Hero, Relay, Tools, Turns } from './sections'

/// The head for the landing page in one language: its title and description, plus links to
/// every language so search engines pair them up.
export function homeHead(lng: Language) {
  const t = i18nFor(lng).t
  return {
    meta: [
      { title: t('meta.title') },
      { name: 'description', content: t('meta.description') },
      { property: 'og:title', content: t('meta.title') },
      { property: 'og:description', content: t('meta.description') },
    ],
    links: [
      { rel: 'canonical', href: SITE + paths[lng] },
      ...languages.map((other) => ({ rel: 'alternate', hrefLang: other, href: SITE + paths[other] })),
    ],
  }
}

export function Home() {
  return (
    <>
      <Nav />
      <main>
        <Hero />
        <div className="h-16 sm:h-24" />
        <Turns />
        <Relay />
        <Tools />
        <Chef />
        <FAQ />
        <CallToAction />
      </main>
      <Footer />
    </>
  )
}
