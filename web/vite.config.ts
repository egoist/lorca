import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import { devtools } from '@tanstack/devtools-vite'

import { tanstackStart } from '@tanstack/react-start/plugin/vite'

import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { cloudflare } from '@cloudflare/vite-plugin'
import { fumadocsMdx } from 'fumadocs-mdx/vite'

import { fetchDesktopRelease } from './src/lib/desktop-release.ts'

/** The model catalog every Device checks for a newer one, at `/models/v1.json`: the CLI's own
 * `crates/models/catalog.json`, minified into `public` so it is served as a static file, whose
 * ETag makes each check a 304 until the catalog itself changes. */
function modelCatalog(): Plugin {
  return {
    name: 'lorca-model-catalog',
    buildStart() {
      const catalog = JSON.parse(readFileSync(fileURLToPath(new URL('../crates/models/catalog.json', import.meta.url)), 'utf8'))
      mkdirSync(fileURLToPath(new URL('./public/models', import.meta.url)), { recursive: true })
      writeFileSync(fileURLToPath(new URL('./public/models/v1.json', import.meta.url)), JSON.stringify(catalog))
    },
  }
}

const config = defineConfig(async ({ command }) => ({
  resolve: { tsconfigPaths: true },
  // The download page's Windows and Linux links, read once per build: a new desktop release
  // reaches the page with the next deploy.
  define: {
    __DESKTOP_RELEASE__: JSON.stringify(await fetchDesktopRelease({ required: command === 'build' })),
  },
  // The docs' generated modules import this at the first server render. Found then, it makes the
  // dev server re-optimize and reload its dependencies mid-render, and that render mixes two copies
  // of React ("Invalid hook call" at Nav). Declared here, it is optimized at startup.
  environments: {
    ssr: { optimizeDeps: { include: ['fumadocs-mdx/runtime/macro'] } },
  },
  plugins: [
    modelCatalog(),
    fumadocsMdx(),
    devtools(),
    cloudflare({ viteEnvironment: { name: 'ssr' } }),
    tailwindcss(),
    tanstackStart(),
    viteReact(),
  ],
}))

export default config
