import { createFileRoute } from '@tanstack/react-router'

import { GithubConnected } from '#/components/site/github'

/// Where the relay sends the user once the Lorca GitHub App is installed: how connecting it went.
export const Route = createFileRoute('/github/connected')({
  head: () => ({ meta: [{ title: 'GitHub · Lorca' }, { name: 'robots', content: 'noindex, nofollow' }] }),
  component: GithubConnected,
})
