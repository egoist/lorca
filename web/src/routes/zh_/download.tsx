import { createFileRoute } from '@tanstack/react-router'

import { Download, downloadHead } from '#/components/site/download'

/// `zh_` keeps this route out of the `/zh` landing page's layout.
export const Route = createFileRoute('/zh_/download')({
  head: () => downloadHead('zh'),
  component: Download,
})
