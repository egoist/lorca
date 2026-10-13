import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import { devtools } from '@tanstack/devtools-vite'

import { tanstackStart } from '@tanstack/react-start/plugin/vite'

import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { cloudflare } from '@cloudflare/vite-plugin'
import { fumadocsMdx } from 'fumadocs-mdx/vite'

import { type DesktopRelease, fetchReleases } from './src/lib/releases.ts'

/** The files every Device checks for a newer copy of: the model catalog at `/models/v1.json` and
 * the marketplace index at `/marketplace/v1.json`, the CLI's own `crates/models/catalog.json` and
 * `crates/cli/marketplace/index.json`, minified into `public` so each is served as a static file,
 * whose ETag makes a check a 304 until the file itself changes. */
const SERVED = [
  ['../crates/models/catalog.json', './public/models/v1.json'],
  ['../crates/cli/marketplace/index.json', './public/marketplace/v1.json'],
]

/** The retired Mac app reads `/desktop-upgrade-url.txt`, and while it holds a link, says a new
 * version is available and opens the link to download it: the newest desktop release's disk
 * image. With no desktop release read, the file is left out. */
const UPGRADE_URL = './public/desktop-upgrade-url.txt'

function servedFiles(desktop: DesktopRelease | null): Plugin {
  return {
    name: 'lorca-served-files',
    buildStart() {
      for (const [from, to] of SERVED) {
        const file = JSON.parse(readFileSync(fileURLToPath(new URL(from, import.meta.url)), 'utf8'))
        const out = fileURLToPath(new URL(to, import.meta.url))
        mkdirSync(dirname(out), { recursive: true })
        writeFileSync(out, JSON.stringify(file))
      }
      const upgradeURL = fileURLToPath(new URL(UPGRADE_URL, import.meta.url))
      if (desktop) writeFileSync(upgradeURL, `${desktop.mac}\n`)
      else rmSync(upgradeURL, { force: true })
    },
  }
}

const config = defineConfig(async ({ command }) => {
  // The download page's Mac, Windows, Linux, and Android links, read once per build: a new release
  // reaches the page with the next deploy.
  const { desktop, android } = await fetchReleases({ required: command === 'build' })
  return {
    resolve: { tsconfigPaths: true },
    define: {
      __DESKTOP_RELEASE__: JSON.stringify(desktop),
      __ANDROID_RELEASE__: JSON.stringify(android),
    },
    // The docs' generated modules import this at the first server render. Found then, it makes the
    // dev server re-optimize and reload its dependencies mid-render, and that render mixes two copies
    // of React ("Invalid hook call" at Nav). Declared here, it is optimized at startup.
    environments: {
      ssr: { optimizeDeps: { include: ['fumadocs-mdx/runtime/macro'] } },
    },
    plugins: [
      servedFiles(desktop),
      fumadocsMdx(),
      devtools(),
      cloudflare({ viteEnvironment: { name: 'ssr' } }),
      tailwindcss(),
      tanstackStart(),
      viteReact(),
    ],
  }
})

export default config
