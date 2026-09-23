import { createFileRoute } from '@tanstack/react-router'

import { Download, downloadHead, latestMacRelease } from '#/components/site/download'

export const Route = createFileRoute('/download')({
  loader: () => latestMacRelease(),
  head: () => downloadHead('en'),
  component: () => <Download release={Route.useLoaderData()} />,
})
