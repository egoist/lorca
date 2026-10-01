// Checks the architecture docs, which every coding session reads: ARCHITECTURE.md, the overview
// read in full, stays within 16 KiB and each subject doc in docs/architecture/ within 24 KiB, so
// one is taken in at a sitting; every subject is listed in the overview's Subjects and every
// listed one exists; and the relative links in the repo's docs resolve, a `#heading` included.
// `bun run check:docs` prints each budgeted doc's size and what is wrong, and fails when anything is.

import { Glob } from "bun"
import { existsSync } from "node:fs"
import { dirname, join, relative } from "node:path"

const ROOT = join(import.meta.dir, "..")
const OVERVIEW = "ARCHITECTURE.md"
const SUBJECTS = "docs/architecture"
const OVERVIEW_BUDGET = 16 * 1024
const SUBJECT_BUDGET = 24 * 1024
/// The docs whose links must resolve.
const LINKED = ["ARCHITECTURE.md", "AGENTS.md", "README.md", "docs/**/*.md", "crates/*/README.md"]

const problems: string[] = []
const texts = new Map<string, string>()

/// A doc's text with LF line endings, so a Windows checkout measures what CI does.
async function text(path: string) {
  if (!texts.has(path)) texts.set(path, (await Bun.file(join(ROOT, path)).text()).replace(/\r\n/g, "\n"))
  return texts.get(path)!
}

async function scan(pattern: string) {
  const paths: string[] = []
  for await (const path of new Glob(pattern).scan({ cwd: ROOT })) paths.push(path.split("\\").join("/"))
  return paths.sort()
}

/// The text outside code fences and inline code, line by line, where links and headings live.
function prose(markdown: string) {
  let fenced = false
  return markdown.split("\n").map((line) => {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced
      return ""
    }
    return fenced ? "" : line.replace(/`[^`]*`/g, "")
  })
}

/// The anchors GitHub gives a doc's headings: lowercase, punctuation dropped, spaces as hyphens,
/// and a repeated one numbered.
function anchors(markdown: string) {
  const seen = new Map<string, number>()
  const out = new Set<string>()
  let fenced = false
  for (const line of markdown.split("\n")) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced
    const heading = !fenced && /^#{1,6}\s+(.+?)\s*#*\s*$/.exec(line)
    if (!heading) continue
    const words = heading[1].replace(/\[([^\]]*)\]\([^)]*\)/g, "$1").replace(/[`*_]/g, "")
    const slug = words.toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, "").replace(/\s/g, "-")
    const count = seen.get(slug) ?? 0
    seen.set(slug, count + 1)
    out.add(count === 0 ? slug : `${slug}-${count}`)
  }
  return out
}

/// The relative links in a doc, with the line each is on.
function links(markdown: string) {
  const found: { target: string; line: number }[] = []
  prose(markdown).forEach((line, index) => {
    for (const match of line.matchAll(/!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g)) {
      const target = match[1]
      if (/^[a-z][a-z0-9+.-]*:/i.test(target) || target.startsWith("/")) continue
      found.push({ target, line: index + 1 })
    }
  })
  return found
}

const docs = [...new Set((await Promise.all(LINKED.map(scan))).flat())]
for (const doc of docs) {
  for (const { target, line } of links(await text(doc))) {
    const [file, anchor] = target.split("#", 2)
    const path = file ? relative(ROOT, join(ROOT, dirname(doc), decodeURIComponent(file))).split("\\").join("/") : doc
    if (!existsSync(join(ROOT, path))) {
      problems.push(`${doc}:${line}: ${target} does not exist`)
      continue
    }
    if (anchor && path.endsWith(".md") && !anchors(await text(path)).has(anchor.toLowerCase())) {
      problems.push(`${doc}:${line}: ${path} has no heading #${anchor}`)
    }
  }
}

/// The text of a doc's `## <title>` section, up to the next heading of its level or above.
function section(markdown: string, title: string) {
  const lines = markdown.split("\n")
  const start = lines.findIndex((line) => line.trim() === `## ${title}`)
  if (start < 0) return undefined
  const end = lines.findIndex((line, index) => index > start && /^#{1,2}\s/.test(line))
  return lines.slice(start + 1, end < 0 ? undefined : end).join("\n")
}

// Every subject is in the overview's Subjects section; a link to it elsewhere in the overview
// does not count.
const subjects = await scan(`${SUBJECTS}/*.md`)
const table = section(await text(OVERVIEW), "Subjects")
if (table === undefined) problems.push(`${OVERVIEW} has no ## Subjects section`)
const listed = new Set(
  links(table ?? "")
    .map(({ target }) => target.split("#")[0])
    .filter((target) => target.startsWith(`${SUBJECTS}/`)),
)
for (const subject of subjects) {
  if (!listed.has(subject)) problems.push(`${subject} is not listed in ${OVERVIEW}'s Subjects`)
}

const kib = (bytes: number) => `${(bytes / 1024).toFixed(1)} KiB`
for (const [doc, budget] of [[OVERVIEW, OVERVIEW_BUDGET] as const, ...subjects.map((subject) => [subject, SUBJECT_BUDGET] as const)]) {
  const bytes = Buffer.byteLength(await text(doc))
  console.log(`${doc.padEnd(40)} ${kib(bytes).padStart(9)} of ${kib(budget)}`)
  if (bytes > budget) problems.push(`${doc} is ${kib(bytes)}, over its ${kib(budget)}: split it by subject, and list the new doc in ${OVERVIEW}`)
}

if (problems.length) {
  console.log()
  for (const problem of problems) console.log(problem)
  process.exit(1)
}
