import { llms, loader } from 'fumadocs-core/source'
import { lucideIconsPlugin } from 'fumadocs-core/source/lucide-icons'
import { defineDocs } from 'fumadocs-mdx/macro'

export const docsRoute = '/docs'

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
  plugins: [lucideIconsPlugin()],
})

/// The same pages as Markdown, for `/llms.txt`, `/llms-full.txt`, and `/docs/<page>.md`.
export const docsLlms = llms(source, {
  renderPage: async (page) => `# ${page.data.title} (${page.url})

${await page.data.getText('processed')}`,
})

/// `/docs/bots` → `/docs/bots.md`; the index is `/docs/index.md`.
export function markdownUrl(slugs: string[]) {
  return `${docsRoute}/${slugs.length === 0 ? 'index' : slugs.join('/')}.md`
}

export function slugsOfMarkdownUrl(segments: string[]) {
  const slugs = [...segments]
  if (slugs.length > 0) slugs[slugs.length - 1] = slugs[slugs.length - 1].replace(/\.md$/, '')
  return slugs.length === 1 && slugs[0] === 'index' ? [] : slugs
}
