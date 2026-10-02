// Checks a change to what lorca.app serves Devices: the model catalog (crates/models/catalog.json)
// and the marketplace index (crates/cli/marketplace/index.json) each move their `updated` time
// forward whenever anything else in them changes, since a Device keeps whichever copy has the later
// time and would never take a changed one under the same time. Compares with `CATALOG_BASE`
// (default origin/main). `bun run check:catalog` runs it locally before a site deploy.

import { $ } from "bun"
import { join } from "node:path"

const ROOT = join(import.meta.dir, "..")
const FILES = [
  ["crates/models/catalog.json", "model catalog"],
  ["crates/cli/marketplace/index.json", "marketplace index"],
]
const base = process.env.CATALOG_BASE || "origin/main"

/// Everything but the time.
const content = (file: Record<string, unknown>) => JSON.stringify({ ...file, updated: null })

let stale = false
for (const [path, name] of FILES) {
  const current = JSON.parse(await Bun.file(join(ROOT, path)).text())
  const shown = await $`git show ${`${base}:${path}`}`.cwd(ROOT).quiet().nothrow()
  if (shown.exitCode !== 0) {
    console.log(`${base} has no ${path}; nothing to compare.`)
    continue
  }
  const previous = JSON.parse(shown.stdout.toString())
  // A file that had no time there has any time it has now.
  const before = previous.updated ?? ""
  if (content(current) === content(previous)) {
    console.log(`The ${name} is as it is at ${base}.`)
  } else if (current.updated > before) {
    console.log(`The ${name} changed since ${base}, and its updated time moved from ${previous.updated ?? "none"} to ${current.updated}.`)
  } else {
    const now = new Date().toISOString().replace(/\.\d+Z$/, "Z")
    console.error(`${path} changed since ${base}, but its "updated" (${current.updated}) is not later than ${before} there.`)
    console.error(`Devices keep the ${name} with the later time, so set it to now: "updated": "${now}"`)
    stale = true
  }
}
if (stale) process.exit(1)
