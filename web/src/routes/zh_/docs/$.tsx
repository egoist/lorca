import { createFileRoute } from '@tanstack/react-router'

import { DocsPage, docsHead, loadDocsPage } from '#/components/docs/page'

/// `zh_` keeps this route out of the `/zh` landing page's layout.
export const Route = createFileRoute('/zh_/docs/$')({
  loader: ({ params }) => loadDocsPage('zh', params._splat),
  head: ({ loaderData }) => docsHead('zh', loaderData),
  component: () => <DocsPage lang="zh" data={Route.useLoaderData()} />,
})
