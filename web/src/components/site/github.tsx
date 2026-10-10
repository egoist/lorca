import { Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { I18nextProvider, useTranslation } from 'react-i18next'

import { Button } from '#/components/ui/button'
import { i18nFor, type Language, paths } from '#/i18n'
import { Header } from './template'

/// Where the relay sends the user after GitHub's install of the Lorca GitHub App and its
/// sign-in: https://lorca.app/github/connected?status=<status>[&account=<login>]. The relay has
/// bound the installation to the account (or said why not) before it gets here; this page only
/// says how it went, in the browser's language, since the address is the same for everyone.

const STATUSES = ['connected', 'not_yours', 'expired', 'requested', 'failed'] as const
type Status = (typeof STATUSES)[number]

const action = 'h-12 rounded-full px-7 text-base'

export function GithubConnected() {
  const [lng, setLng] = useState<Language | null>(null)
  const [status, setStatus] = useState<Status>('failed')
  const [account, setAccount] = useState('')

  useEffect(() => {
    const preferred = (navigator.languages?.[0] ?? navigator.language ?? '').toLowerCase()
    setLng(preferred.startsWith('zh') ? 'zh' : 'en')
    const query = new URLSearchParams(window.location.search)
    const given = query.get('status') as Status | null
    setStatus(given && STATUSES.includes(given) ? given : 'failed')
    setAccount(query.get('account') ?? '')
  }, [])

  useEffect(() => {
    if (lng) document.documentElement.lang = lng === 'zh' ? 'zh-Hans' : 'en'
  }, [lng])

  return (
    <I18nextProvider i18n={i18nFor(lng ?? 'en')}>
      <div className="flex min-h-svh flex-col">
        <Header language={lng} onLanguage={setLng} />
        <main className="flex-1 px-4 pt-14 pb-20 sm:pt-20">
          <div className="mx-auto max-w-2xl">
            {/* Nothing in a language until the browser says which. */}
            {lng ? <Outcome status={status} account={account} /> : <div className="panel h-56 animate-pulse bg-foreground/[0.03]" aria-busy="true" />}
          </div>
        </main>
      </div>
    </I18nextProvider>
  )
}

function Outcome({ status, account }: { status: Status; account: string }) {
  const { t, i18n } = useTranslation()
  return (
    <section className="panel px-7 py-10 text-center sm:px-10">
      <h1 className="text-2xl font-semibold tracking-tight">{t(`github.${status}.title`)}</h1>
      <p className="mt-3 leading-relaxed text-muted-foreground">{t(`github.${status}.body`, { account: account || 'GitHub' })}</p>
      {status === 'connected' ? (
        <p className="mt-3 leading-relaxed text-muted-foreground">{t('github.back')}</p>
      ) : (
        <Button asChild size="lg" variant="outline" className={`${action} mt-8 bg-transparent shadow-none dark:bg-transparent`}>
          <Link to={paths[i18n.language as Language]}>{t('github.home')}</Link>
        </Button>
      )}
    </section>
  )
}
