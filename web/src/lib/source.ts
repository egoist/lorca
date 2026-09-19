import { defineI18n } from 'fumadocs-core/i18n'
import { llms, loader } from 'fumadocs-core/source'
import { lucideIconsPlugin } from 'fumadocs-core/source/lucide-icons'
import { defineDocs } from 'fumadocs-mdx/macro'

export const docsRoute = '/docs'

/// The docs' languages, as the site has them: English at `/docs`, Chinese at `/zh/docs`. A page
/// is `bots.mdx` with `bots.zh.mdx` beside it; one with no translation falls back to English.
export const docsI18n = defineI18n({
  defaultLanguage: 'en',
  languages: ['en', 'zh'],
  hideLocale: 'default-locale',
})

/// The product docs: MDX under `web/content/docs`, compiled by the `fumadocsMdx` Vite plugin.
export const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    async: true,
    postprocess: { includeProcessedMarkdown: true },
  },
})

export const source = loader({
  source: docs.toFumadocsSource(),
  baseUrl: docsRoute,
  i18n: docsI18n,
  plugins: [lucideIconsPlugin()],
})

/// The same pages as Markdown, for `/llms.txt`, `/llms-full.txt`, and `/docs/<page>.md`.
export const docsLlms = llms(source, {
  renderPage: async (page) => `# ${page.data.title} (${page.url})

${await page.data.getText('processed')}`,
})

/// `/docs/bots` → `/docs/bots.md`; the index is `/docs/index.md`. Chinese pages sit under `/zh`.
export function markdownUrl(slugs: string[], lang: string = docsI18n.defaultLanguage) {
  const prefix = lang === docsI18n.defaultLanguage ? '' : `/${lang}`
  return `${prefix}${docsRoute}/${slugs.length === 0 ? 'index' : slugs.join('/')}.md`
}

export function slugsOfMarkdownUrl(segments: string[]) {
  const slugs = [...segments]
  if (slugs.length > 0) slugs[slugs.length - 1] = slugs[slugs.length - 1].replace(/\.md$/, '')
  return slugs.length === 1 && slugs[0] === 'index' ? [] : slugs
}
