import { createFileRoute, notFound } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useFumadocsLoader } from 'fumadocs-core/source/client'
import { DocsLayout } from 'fumadocs-ui/layouts/docs'
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
  MarkdownCopyButton,
} from 'fumadocs-ui/layouts/docs/page'
import { RootProvider } from 'fumadocs-ui/provider/tanstack'
import { Suspense, use } from 'react'

import { useMDXComponents } from '#/components/mdx'
import { Logo } from '#/components/site/logo'
import { DOWNLOAD, SITE } from '#/components/site/nav'
import { docs, markdownUrl, source } from '#/lib/source'

const serverLoader = createServerFn({ method: 'GET' })
  .validator((slugs: string[]) => slugs)
  .handler(async ({ data: slugs }) => {
    const page = source.getPage(slugs)
    if (!page) throw notFound()
    return {
      path: page.path,
      url: page.url,
      title: page.data.title,
      description: page.data.description,
      markdownUrl: markdownUrl(page.slugs),
      pageTree: await source.serializePageTree(source.getPageTree()),
    }
  })

export const Route = createFileRoute('/docs/$')({
  component: Page,
  loader: async ({ params }) => {
    const data = await serverLoader({ data: params._splat?.split('/').filter(Boolean) ?? [] })
    await docs.getPage(data.path)?.preload()
    return data
  },
  head: ({ loaderData }) => {
    if (!loaderData) return {}
    const title = `${loaderData.title} · Lorca Docs`
    return {
      meta: [
        { title },
        { name: 'description', content: loaderData.description },
        { property: 'og:title', content: title },
        { property: 'og:description', content: loaderData.description },
      ],
      links: [{ rel: 'canonical', href: SITE + loaderData.url }],
    }
  },
})

function Content({ path, markdownUrl }: { path: string; markdownUrl: string }) {
  const page = docs.getPage(path)
  if (!page) throw new Error(`unknown page: ${path}`)

  const { toc } = use(page.load())
  const MDX = page.body

  return (
    <DocsPage toc={toc}>
      <DocsTitle>{page.title}</DocsTitle>
      <DocsDescription>{page.description}</DocsDescription>
      <div className="-mt-4 flex flex-row items-center gap-2 border-b pb-6">
        <MarkdownCopyButton markdownUrl={markdownUrl} />
      </div>
      <DocsBody>
        <MDX components={useMDXComponents()} />
      </DocsBody>
    </DocsPage>
  )
}

function Page() {
  const { path, pageTree, markdownUrl } = useFumadocsLoader(Route.useLoaderData())

  /// The site's colors follow the system, so the docs do too: the provider keeps `.dark` on
  /// <html> in step with it and the layout offers no switch.
  return (
    <RootProvider theme={{ defaultTheme: 'system', enableSystem: true, forcedTheme: 'system' }}>
      <DocsLayout
        tree={pageTree}
        themeSwitch={{ enabled: false }}
        nav={{
          title: (
            <>
              <Logo className="size-6" />
              <span className="font-semibold tracking-tight">Lorca</span>
              <span className="text-fd-muted-foreground">Docs</span>
            </>
          ),
          url: '/',
        }}
        links={[{ text: 'Download for Mac', url: DOWNLOAD }]}
      >
        <Suspense>
          <Content path={path} markdownUrl={markdownUrl} />
        </Suspense>
      </DocsLayout>
    </RootProvider>
  )
}
