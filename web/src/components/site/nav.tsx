import { Button } from '#/components/ui/button'
import { Logo } from './logo'

/// Where the installer lives. macOS ships first; the same button will offer the other platforms.
export const DOWNLOAD = '/download'
export const SITE = 'https://lorca.app'

const links = [
  { href: '#turns', label: 'Turns' },
  { href: '#relay', label: 'Relay' },
  { href: '#tools', label: 'Tools' },
  { href: '#faq', label: 'FAQ' },
]

export function Nav() {
  return (
    <header className="sticky top-0 z-40 border-b bg-background/70 backdrop-blur-xl">
      <div className="mx-auto flex h-14 max-w-6xl items-center justify-between px-5">
        <a href="#top" className="flex items-center gap-2.5 font-semibold tracking-tight">
          <Logo className="size-7" />
          Lorca
        </a>
        <nav className="hidden items-center gap-7 text-sm text-muted-foreground md:flex">
          {links.map((link) => (
            <a key={link.href} href={link.href} className="transition-colors hover:text-foreground">
              {link.label}
            </a>
          ))}
        </nav>
        <Button asChild size="sm" className="rounded-full px-4">
          <a href={DOWNLOAD}>Download for Mac</a>
        </Button>
      </div>
    </header>
  )
}
