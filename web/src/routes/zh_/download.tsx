import { createFileRoute } from '@tanstack/react-router'

import { Download, downloadHead, latestMacRelease } from '#/components/site/download'

/// `zh_` keeps this route out of the `/zh` landing page's layout.
export const Route = createFileRoute('/zh_/download')({
  loader: () => latestMacRelease(),
  head: () => downloadHead('zh'),
  component: () => <Download release={Route.useLoaderData()} />,
})
