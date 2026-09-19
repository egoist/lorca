import { HeadContent, Scripts, createRootRoute, useLocation } from '@tanstack/react-router'
import { I18nextProvider } from 'react-i18next'

import { htmlLang, i18nFor, languageOf } from '#/i18n'

import appCss from '../styles.css?url'

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      { name: 'theme-color', content: '#fafafb', media: '(prefers-color-scheme: light)' },
      { name: 'theme-color', content: '#0f0f12', media: '(prefers-color-scheme: dark)' },
      { property: 'og:type', content: 'website' },
      { property: 'og:image', content: '/screens/group.png' },
      { name: 'twitter:card', content: 'summary_large_image' },
    ],
    links: [
      { rel: 'preconnect', href: 'https://fonts.googleapis.com' },
      { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossOrigin: 'anonymous' },
      {
        rel: 'stylesheet',
        href: 'https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Instrument+Serif:ital@0;1&display=swap',
      },
      { rel: 'stylesheet', href: appCss },
      { rel: 'icon', href: '/icon.svg', type: 'image/svg+xml' },
    ],
  }),
  shellComponent: RootDocument,
})

function RootDocument({ children }: { children: React.ReactNode }) {
  const lng = languageOf(useLocation({ select: (location) => location.pathname }))
  return (
    <html lang={htmlLang[lng]}>
      <head>
        <HeadContent />
      </head>
      <body>
        <I18nextProvider i18n={i18nFor(lng)}>{children}</I18nextProvider>
        <Scripts />
      </body>
    </html>
  )
}
