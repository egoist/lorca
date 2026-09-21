import { useTranslation } from 'react-i18next'

/// Three computers, one relay: sealed envelopes travel along the lines, and the box in the middle
/// only ever holds ciphertext. Machine names come from `relay.machines`, in this order.
const machines = [
  { bots: 'Nova', x: 90, y: 60 },
  { bots: 'Patch · Scout', x: 610, y: 60 },
  { bots: 'Ember', x: 350, y: 300 },
]
const relay = { x: 350, y: 170 }

export function RelayDiagram() {
  const { t } = useTranslation()
  const names = t('relay.machines', { returnObjects: true })
  return (
    <svg viewBox="0 0 700 360" className="w-full" role="img" aria-label={t('relay.alt')}>
      <defs>
        <linearGradient id="wire" gradientUnits="userSpaceOnUse" x1="0" x2="700">
          <stop offset="0" stopColor="#6b66f5" />
          <stop offset="1" stopColor="#2eb3dc" />
        </linearGradient>
      </defs>
      <defs>
        <pattern id="grid" width="28" height="28" patternUnits="userSpaceOnUse">
          <path d="M 28 0 L 0 0 0 28" fill="none" stroke="rgba(255,255,255,0.06)" strokeWidth="1" />
        </pattern>
      </defs>
      <rect width="700" height="360" fill="url(#grid)" />
      {machines.map((m, i) => {
        const path = `M ${m.x} ${m.y} L ${relay.x} ${relay.y}`
        return (
          <g key={i}>
            <path d={path} stroke="url(#wire)" strokeOpacity="0.35" strokeWidth="1.5" strokeDasharray="4 6" />
            <g style={{ offsetPath: `path('${path}')`, animation: `travel ${3.2 + i * 0.6}s linear infinite`, animationDelay: `${i * 0.9}s`, offsetRotate: '0deg' }}>
              <rect x="-9" y="-6" width="18" height="12" rx="2.5" fill="#0f0f12" stroke="#8b8bff" strokeWidth="1.2" />
              <path d="M-9 -6 L0 1 L9 -6" fill="none" stroke="#8b8bff" strokeWidth="1.2" />
            </g>
            <g style={{ offsetPath: `path('${path}')`, animation: `travel ${3.2 + i * 0.6}s linear infinite reverse`, animationDelay: `${i * 0.9 + 1.5}s`, offsetRotate: '0deg' }}>
              <rect x="-9" y="-6" width="18" height="12" rx="2.5" fill="#0f0f12" stroke="#5cd6cd" strokeWidth="1.2" />
              <path d="M-9 -6 L0 1 L9 -6" fill="none" stroke="#5cd6cd" strokeWidth="1.2" />
            </g>
          </g>
        )
      })}
      {machines.map((m, i) => (
        <g key={i} transform={`translate(${m.x - 70} ${m.y - 26})`}>
          <rect width="140" height="52" rx="12" fill="#18181b" stroke="rgba(255,255,255,0.14)" />
          <circle cx="20" cy="26" r="4" fill="#34d399" />
          <text x="34" y="23" fill="#f4f4f5" fontSize="13" fontWeight="600" fontFamily="Inter, system-ui">{names[i]}</text>
          <text x="34" y="39" fill="#a1a1aa" fontSize="11" fontFamily="Inter, system-ui">{m.bots}</text>
        </g>
      ))}
      <g transform={`translate(${relay.x - 84} ${relay.y - 30})`}>
        <rect width="168" height="60" rx="14" fill="#0f0f12" stroke="url(#wire)" strokeWidth="1.5" />
        <text x="84" y="26" textAnchor="middle" fill="#f4f4f5" fontSize="13" fontWeight="600" fontFamily="Inter, system-ui">{t('relay.name')}</text>
        <text x="84" y="44" textAnchor="middle" fill="#a1a1aa" fontSize="11" fontFamily="SF Mono, Menlo, monospace">a7f2…c9e1 · {t('relay.ciphertext')}</text>
      </g>
    </svg>
  )
}
