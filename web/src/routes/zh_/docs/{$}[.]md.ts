import { createFileRoute, notFound } from '@tanstack/react-router'

import { docsLlms, slugsOfMarkdownUrl, source } from '#/lib/source'

export const Route = createFileRoute('/zh_/docs/{$}.md')({
  server: {
    handlers: {
      GET: async ({ params }) => {
        const page = source.getPage(slugsOfMarkdownUrl(params._splat?.split('/') ?? []), 'zh')
        if (!page) throw notFound()
        return new Response(await docsLlms.page(page), {
          headers: { 'Content-Type': 'text/markdown; charset=utf-8' },
        })
      },
    },
  },
})
