import { createTokenizer } from '@orama/tokenizers/mandarin'
import { createFileRoute } from '@tanstack/react-router'
import { createFromSource } from 'fumadocs-core/search/server'

import { source } from '#/lib/source'

/// One index per language. Orama has no stemmer for Chinese, so `zh` brings its own tokenizer
/// and matches exactly.
const server = createFromSource(source, {
  localeMap: {
    en: { language: 'english' },
    zh: { components: { tokenizer: createTokenizer() }, search: { threshold: 0, tolerance: 0 } },
  },
})

export const Route = createFileRoute('/api/search')({
  server: { handlers: { GET: async ({ request }) => server.GET(request) } },
})
