// Checks a change to the model catalog: crates/models/catalog.json moves its `updated` time
// forward whenever anything else in it changes, since a Device keeps whichever catalog has the
// later time and would never take a changed one under the same time. Compares with `CATALOG_BASE`
// (default origin/main). `bun run check:catalog` runs it locally before a site deploy.

import { $ } from "bun"
import { join } from "node:path"

const ROOT = join(import.meta.dir, "..")
const CATALOG = "crates/models/catalog.json"
const base = process.env.CATALOG_BASE || "origin/main"

const current = JSON.parse(await Bun.file(join(ROOT, CATALOG)).text())
const shown = await $`git show ${`${base}:${CATALOG}`}`.cwd(ROOT).quiet().nothrow()
if (shown.exitCode !== 0) {
  console.log(`${base} has no ${CATALOG}; nothing to compare.`)
  process.exit(0)
}
const previous = JSON.parse(shown.stdout.toString())

/// Everything but the time.
const content = (catalog: Record<string, unknown>) => JSON.stringify({ ...catalog, updated: null })

if (content(current) === content(previous)) {
  console.log(`The model catalog is as it is at ${base}.`)
} else if (current.updated > previous.updated) {
  console.log(`The model catalog changed since ${base}, and its updated time moved from ${previous.updated} to ${current.updated}.`)
} else {
  const now = new Date().toISOString().replace(/\.\d+Z$/, "Z")
  console.error(`${CATALOG} changed since ${base}, but its "updated" (${current.updated}) is not later than ${previous.updated} there.`)
  console.error(`Devices keep the catalog with the later time, so set it to now: "updated": "${now}"`)
  process.exit(1)
}
