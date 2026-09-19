import { createFileRoute } from '@tanstack/react-router'

import { DocsPage, docsHead, loadDocsPage } from '#/components/docs/page'

export const Route = createFileRoute('/docs/$')({
  loader: ({ params }) => loadDocsPage('en', params._splat),
  head: ({ loaderData }) => docsHead('en', loaderData),
  component: () => <DocsPage lang="en" data={Route.useLoaderData()} />,
})
