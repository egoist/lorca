import { createFileRoute } from '@tanstack/react-router'

import { SharedTemplate } from '#/components/site/template'

/// A bot someone shared: /t/<id>#<key>. The page opens it in the browser.
export const Route = createFileRoute('/t/$id')({
  head: () => ({
    meta: [
      { title: 'Lorca' },
      // The address carries a key: no search engine, and no Referer to the next site.
      { name: 'robots', content: 'noindex, nofollow' },
      { name: 'referrer', content: 'no-referrer' },
    ],
  }),
  component: () => <SharedTemplate id={Route.useParams().id} />,
})
