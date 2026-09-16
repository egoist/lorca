/// A pixel-gradient backdrop: a 48×27 PNG (generated offline from a smooth color field, in
/// `public/pixels`) scaled up with crisp edges. Cheap to paint at any size.
export function Pixels({ seed, className = '' }: { seed: 2 | 8 | 11; className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={className}
      style={{
        backgroundImage: `url(/pixels/${seed}.png)`,
        backgroundSize: 'cover',
        backgroundPosition: 'center',
        imageRendering: 'pixelated',
      }}
    />
  )
}
