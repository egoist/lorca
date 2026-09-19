import { createFileRoute } from '@tanstack/react-router'

import { docsLlms } from '#/lib/source'

const headers = { 'Content-Type': 'text/plain; charset=utf-8' }

export const Route = createFileRoute('/zh_/llms.txt')({
  server: { handlers: { GET: async () => new Response(await docsLlms.index('zh'), { headers }) } },
})
