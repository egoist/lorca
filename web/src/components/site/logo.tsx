export function Logo({ className = 'size-7' }: { className?: string }) {
  return (
    <svg viewBox="0 0 512 512" className={className} aria-hidden="true">
      <defs>
        <linearGradient id="tb-plate" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#6b66f5" />
          <stop offset="0.55" stopColor="#3d85f0" />
          <stop offset="1" stopColor="#2eb3dc" />
        </linearGradient>
      </defs>
      <rect x="26" y="26" width="460" height="460" rx="104" fill="url(#tb-plate)" />
      <path d="M256 160v-40" stroke="#fff" strokeWidth="16" strokeLinecap="round" />
      <circle cx="256" cy="103" r="21" fill="#fff" />
      <rect x="118" y="156" width="276" height="224" rx="74" fill="#fff" />
      <circle cx="202" cy="262" r="22" fill="#4576f2" />
      <circle cx="310" cy="262" r="22" fill="#4576f2" />
    </svg>
  )
}
