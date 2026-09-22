import { Laptop, Lock, Smartphone } from 'lucide-react'
import { useTranslation } from 'react-i18next'

/// Your computer, the relay, your phone: a pulse of light runs along the connector from one card
/// to the next, and the relay in the middle can only pass it along. The row stacks on a phone.
export function RelayDiagram() {
  const { t } = useTranslation()
  return (
    <div role="img" aria-label={t('relay.alt')} className="flex flex-col items-center sm:flex-row sm:items-stretch">
      <Node icon={Laptop} title={t('relay.nodes.computer.title')} body={t('relay.nodes.computer.body')} />
      <Wire />
      <Node icon={Lock} title={t('relay.nodes.relay.title')} body={t('relay.nodes.relay.body')} accent />
      <Wire second />
      <Node icon={Smartphone} title={t('relay.nodes.phone.title')} body={t('relay.nodes.phone.body')} />
    </div>
  )
}

function Node({
  icon: Icon,
  title,
  body,
  accent = false,
}: {
  icon: typeof Laptop
  title: string
  body: string
  accent?: boolean
}) {
  return (
    <div className="flex w-full max-w-64 flex-none flex-col justify-center rounded-2xl border border-white/50 bg-white/85 px-5 py-5 text-center shadow-xl backdrop-blur-md sm:w-52 dark:border-white/10 dark:bg-zinc-900/80">
      <span
        className={`mx-auto flex size-11 items-center justify-center rounded-full ${
          accent ? 'bg-linear-to-br from-violet to-cyan text-white' : 'bg-foreground/[0.06] text-foreground'
        }`}
      >
        <Icon className="size-5" strokeWidth={1.75} />
      </span>
      <p className="mt-3 font-semibold">{title}</p>
      <p className="mt-1 text-sm text-muted-foreground">{body}</p>
    </div>
  )
}

/// The connector between two cards. The pulse crosses in the first half of the cycle, so the
/// second wire, half a cycle behind, picks it up where the first one left it.
function Wire({ second = false }: { second?: boolean }) {
  return (
    <div
      className="wire relative h-16 w-0.5 flex-none overflow-hidden rounded-full sm:h-0.5 sm:w-auto sm:flex-1 sm:self-center"
      style={second ? { animationDelay: '-2.5s' } : undefined}
    />
  )
}
