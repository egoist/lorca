import { createFileRoute } from '@tanstack/react-router'

import { Download, downloadHead } from '#/components/site/download'

export const Route = createFileRoute('/download')({
  head: () => downloadHead('en'),
  component: Download,
})
