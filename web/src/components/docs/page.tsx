import { notFound, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { zhCN } from '@fumadocs/language/zh-cn'
import { useFumadocsLoader } from 'fumadocs-core/source/client'
import { i18nProvider, uiTranslations } from 'fumadocs-ui/i18n'
import { DocsLayout } from 'fumadocs-ui/layouts/docs'
import {
  DocsBody,
  DocsDescription,
  DocsPage as Article,
  DocsTitle,
  MarkdownCopyButton,
} from 'fumadocs-ui/layouts/docs/page'
import { RootProvider } from 'fumadocs-ui/provider/tanstack'
import { Suspense, use } from 'react'

import { useMDXComponents } from '#/components/mdx'
import { Logo } from '#/components/site/logo'
import { DOWNLOAD, SITE } from '#/components/site/nav'
import { i18nFor, type Language, names, paths } from '#/i18n'
import { docs, docsI18n, docsRoute, markdownUrl, source } from '#/lib/source'

const translations = docsI18n.translations().extend(uiTranslations()).preset('zh', zhCN())
const locales = docsI18n.languages.map((locale) => ({ locale, name: names[locale] }))

const serverLoader = createServerFn({ method: 'GET' })
  .validator((params: { slugs: string[]; lang: Language }) => params)
  .handler(async ({ data: { slugs, lang } }) => {
    const page = source.getPage(slugs, lang)
    if (!page) throw notFound()
    return {
      path: page.path,
      url: page.url,
      slugs: page.slugs,
      title: page.data.title,
      description: page.data.description,
      markdownUrl: markdownUrl(page.slugs, lang),
      pageTree: await source.serializePageTree(source.getPageTree(lang)),
    }
  })

export type DocsData = Awaited<ReturnType<typeof serverLoader>>

/// The loader of a docs route in one language: `/docs/$` and `/zh/docs/$` differ in this alone.
export async function loadDocsPage(lang: Language, splat: string | undefined) {
  const data = await serverLoader({ data: { slugs: splat?.split('/').filter(Boolean) ?? [], lang } })
  await docs.getPage(data.path)?.preload()
  return data
}

/// Where a page lives in a language: `/docs/bots` and `/zh/docs/bots`.
function urlIn(lang: Language, slugs: string[]) {
  const prefix = lang === docsI18n.defaultLanguage ? '' : `/${lang}`
  return [prefix + docsRoute, ...slugs].join('/')
}

export function docsHead(lang: Language, data: DocsData | undefined) {
  if (!data) return {}
  const title = `${data.title} · ${i18nFor(lang).t('docs.title')}`
  return {
    meta: [
      { title },
      { name: 'description', content: data.description },
      { property: 'og:title', content: title },
      { property: 'og:description', content: data.description },
    ],
    links: [
      { rel: 'canonical', href: SITE + data.url },
      ...docsI18n.languages.map((other) => ({
        rel: 'alternate',
        hrefLang: other,
        href: SITE + urlIn(other, data.slugs),
      })),
    ],
  }
}

function Content({ path, markdownUrl }: { path: string; markdownUrl: string }) {
  const page = docs.getPage(path)
  if (!page) throw new Error(`unknown page: ${path}`)

  const { toc } = use(page.load())
  const MDX = page.body

  return (
    <Article toc={toc}>
      <DocsTitle>{page.title}</DocsTitle>
      <DocsDescription>{page.description}</DocsDescription>
      <div className="-mt-4 flex flex-row items-center gap-2 border-b pb-6">
        <MarkdownCopyButton markdownUrl={markdownUrl} />
      </div>
      <DocsBody>
        <MDX components={useMDXComponents()} />
      </DocsBody>
    </Article>
  )
}

export function DocsPage({ lang, data }: { lang: Language; data: DocsData }) {
  const { path, pageTree, markdownUrl, slugs } = useFumadocsLoader(data)
  const t = i18nFor(lang).t
  const navigate = useNavigate()

  /// The site's colors follow the system, so the docs do too: the provider keeps `.dark` on
  /// <html> in step with it and the layout offers no switch.
  return (
    <RootProvider
      theme={{ defaultTheme: 'system', enableSystem: true, forcedTheme: 'system' }}
      i18n={{
        ...i18nProvider(translations, lang),
        locales,
        // The layout's language picker: the same page in the language picked.
        onLocaleChange: (locale) => navigate({ href: urlIn(locale as Language, slugs) }),
      }}
    >
      <DocsLayout
        tree={pageTree}
        themeSwitch={{ enabled: false }}
        i18n
        nav={{
          title: (
            <>
              <Logo className="size-6" />
              <span className="font-semibold tracking-tight">Lorca</span>
              <span className="text-fd-muted-foreground">{t('nav.docs')}</span>
            </>
          ),
          url: paths[lang],
        }}
        links={[{ text: t('nav.download'), url: DOWNLOAD }]}
      >
        <Suspense>
          <Content path={path} markdownUrl={markdownUrl} />
        </Suspense>
      </DocsLayout>
    </RootProvider>
  )
}
