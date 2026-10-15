// Syncs the model catalog (crates/models/catalog.json) with its upstream sources: models.dev for
// each listed model's rates, long-context tiers, window, and output cap (and an OpenCode model's
// thinking levels), and OpenCode's Zen and Go endpoint tables for each OpenCode model's wire. Which
// models are listed, their names, thinking levels, and review models stay curated: upstream models
// the catalog lacks, Codex's ChatGPT lineup, and listed models upstream no longer has go into the
// report for a person to act on. A change moves "updated" to now. `bun run sync:models [--report
// <file>] [--new <file>]`, where --new writes just the upstream models the catalog lacks, one per
// line; the Models sync workflow runs it every day at 00:00 UTC, opens a pull request with the
// report on a change, and keeps an issue open with the new models while there are any.

import { join } from "node:path"

const ROOT = join(import.meta.dir, "..")
const CATALOG = join(ROOT, "crates/models/catalog.json")

/// The catalog's providers, by the models.dev provider that prices them.
const MODELS_DEV: Record<string, string> = {
  deepseek: "deepseek",
  anthropic: "anthropic",
  chatgpt: "openai",
  grok: "xai",
  opencode: "opencode",
  "opencode-go": "opencode-go",
}

/// Fields kept as written although models.dev says otherwise: the Grok sign-in caps output at 64k,
/// DeepSeek documents its cap as 384K and takes every effort on its Anthropic endpoint, Lorca turns
/// thinking off on Claude models models.dev lists no toggle for (Sonnet 5.5's off is
/// between_tools), and the ChatGPT sign-in's levels are Codex's, which have no none.
const KEEP: Record<string, string[]> = {
  "grok/*": ["max_output"],
  "deepseek/*": ["max_output", "levels"],
  "anthropic/claude-opus-5": ["levels"],
  "anthropic/claude-opus-4-8": ["levels"],
  "anthropic/claude-sonnet-5-5": ["levels"],
  "chatgpt/*": ["levels"],
}

/// OpenCode's endpoint tables, by provider.
const OPENCODE_DOCS: Record<string, string> = {
  opencode: "https://opencode.ai/docs/zen/",
  "opencode-go": "https://opencode.ai/docs/go/",
}
const WIRES: Record<string, string> = {
  responses: "responses",
  messages: "messages",
  "chat/completions": "chat-completions",
  systemone: "system-one",
}

const CODEX_MODELS = "https://raw.githubusercontent.com/openai/codex/main/codex-rs/models-manager/models.json"

/// How far back an upstream release counts as new for the report.
const NEW_DAYS = 45

type Rates = { input: number; output: number; cache_read: number; cache_write: number }
type Tier = { input_tokens_above: number; rates: Rates }
type Model = {
  provider: string
  id: string
  name: string
  context_window: number
  max_output: number
  rates: Rates
  tiers?: Tier[]
  levels?: string[]
  wire?: string
}

/// Providers whose models take their thinking levels from models.dev. The others' levels follow
/// how Lorca drives them (the ChatGPT sign-in's from Codex, Sonnet 5.5's off as between_tools),
/// so a difference there is only reported.
const SYNC_LEVELS = ["opencode", "opencode-go"]

/// Lorca's thinking levels, lowest first.
const LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]

/// A model's levels from models.dev's `reasoning_options`: a toggle, or an effort of `none`, is
/// Lorca's `off`, and each other effort it knows is itself. Undefined when models.dev says nothing
/// or offers only a token budget, which no level maps to.
function levelsFrom(options: unknown): string[] | undefined {
  if (!Array.isArray(options)) return undefined
  if (options.length && options.every((o) => o?.type === "budget_tokens")) return undefined
  const levels = new Set<string>()
  for (const option of options) {
    if (option?.type === "toggle") levels.add("off")
    if (option?.type === "effort")
      for (const value of option.values ?? []) {
        const level = value === "none" || value === null ? "off" : value
        if (LEVELS.includes(level)) levels.add(level)
      }
  }
  return LEVELS.filter((level) => levels.has(level))
}
const levelsText = (levels: string[]) => `[${levels.map((level) => `"${level}"`).join(", ")}]`

const args = process.argv.slice(2)
const option = (name: string) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
const reportPath = option("--report")
const newPath = option("--new")

async function json(url: string) {
  const res = await fetch(url)
  if (!res.ok) throw new Error(`${url}: ${res.status}`)
  return res.json()
}

/// Model id → wire from an OpenCode docs page's endpoint table; empty when the page can't be read.
async function opencodeWires(url: string): Promise<Map<string, string>> {
  const wires = new Map<string, string>()
  const res = await fetch(url).catch(() => undefined)
  if (!res?.ok) {
    console.warn(`Could not read ${url}; OpenCode wires are left as they are.`)
    return wires
  }
  const text = (await res.text()).replace(/<script[\s\S]*?<\/script>|<style[\s\S]*?<\/style>/g, "")
  const cells = text
    .split(/<[^>]+>/)
    .map((cell) => cell.trim())
    .filter(Boolean)
  cells.forEach((cell, i) => {
    const endpoint = cell.match(/^https:\/\/opencode\.ai\/inference\/(?:go\/)?v1\/(.+)$/)
    const wire = endpoint && WIRES[endpoint[1]]
    if (wire && /^[\w.-]+$/.test(cells[i - 1])) wires.set(cells[i - 1], wire)
  })
  return wires
}

const num = (n: number) => (Number.isInteger(n) ? n.toFixed(1) : String(n))
const ratesFrom = (cost: Record<string, number>): Rates => ({
  input: cost.input ?? 0,
  output: cost.output ?? 0,
  cache_read: cost.cache_read ?? 0,
  cache_write: cost.cache_write ?? 0,
})
const ratesText = (r: Rates) =>
  `{ "input": ${num(r.input)}, "output": ${num(r.output)}, "cache_read": ${num(r.cache_read)}, "cache_write": ${num(r.cache_write)} }`
const tiersText = (tiers: Tier[]) =>
  `[${tiers.map((t) => `{ "input_tokens_above": ${t.input_tokens_above}, "rates": ${ratesText(t.rates)} }`).join(", ")}]`
const kept = (m: Model, field: string) =>
  [`${m.provider}/${m.id}`, `${m.provider}/*`].some((key) => KEEP[key]?.includes(field))

const [modelsDev, codex, ...wireTables] = await Promise.all([
  json("https://models.dev/api.json"),
  json(CODEX_MODELS).catch((error) => {
    console.warn(`Could not read Codex's models.json: ${error}`)
    return undefined
  }),
  ...Object.values(OPENCODE_DOCS).map(opencodeWires),
])
const wires = Object.fromEntries(Object.keys(OPENCODE_DOCS).map((provider, i) => [provider, wireTables[i]]))

let text = await Bun.file(CATALOG).text()
const catalog: { updated: string; models: Model[] } = JSON.parse(text)
const changes: string[] = []
const notes: string[] = []

/// Replaces a field's line inside one model's block, or adds it after `after`.
function setField(m: Model, field: string, value: string, after?: string) {
  const block = new RegExp(`( {4}\\{\\n {6}"provider": "${m.provider}",\\n {6}"id": "${m.id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}",[\\s\\S]*?\\n {4}\\})`)
  const found = text.match(block)
  if (!found) throw new Error(`No block for ${m.provider}/${m.id}`)
  let body = found[1]
  const line = new RegExp(`\\n {6}"${field}": [^\\n]*?(,?)(?=\\n)`)
  if (line.test(body)) body = body.replace(line, (_, comma) => `\n      "${field}": ${value}${comma}`)
  else if (after) body = body.replace(new RegExp(`(\\n {6}"${after}": [^\\n]*,)`), `$1\n      "${field}": ${value},`)
  else throw new Error(`${m.provider}/${m.id} has no ${field}`)
  text = text.replace(found[1], body)
}

for (const m of catalog.models) {
  const name = `${m.provider}/${m.id}`
  const wire = wires[m.provider]?.get(m.id)
  if (wire && m.wire && wire !== m.wire) {
    setField(m, "wire", `"${wire}"`)
    changes.push(`${name}: wire ${m.wire} → ${wire}`)
  }

  const upstream = modelsDev[MODELS_DEV[m.provider]]?.models?.[m.id]
  if (!upstream) {
    if (m.wire !== "system-one") notes.push(`${name} is not on models.dev (${MODELS_DEV[m.provider]}).`)
    continue
  }
  if (upstream.status === "deprecated") notes.push(`${name} is deprecated on models.dev.`)

  const levels = levelsFrom(upstream.reasoning_options)
  if (levels && m.levels && levelsText(levels) !== levelsText(m.levels)) {
    if (SYNC_LEVELS.includes(m.provider)) {
      setField(m, "levels", levelsText(levels))
      changes.push(`${name}: levels ${levelsText(m.levels)} → ${levelsText(levels)}`)
    } else if (!kept(m, "levels")) {
      notes.push(`${name} takes ${levelsText(m.levels)}; models.dev says ${levelsText(levels)}.`)
    }
  }

  for (const [field, value] of [
    ["context_window", upstream.limit?.context],
    ["max_output", upstream.limit?.output],
  ] as const) {
    if (typeof value === "number" && value > 0 && value !== m[field] && !kept(m, field)) {
      setField(m, field, String(value))
      changes.push(`${name}: ${field} ${m[field]} → ${value}`)
    }
  }

  if (upstream.cost) {
    const rates = ratesFrom(upstream.cost)
    if (ratesText(rates) !== ratesText(m.rates)) {
      setField(m, "rates", ratesText(rates))
      changes.push(`${name}: rates ${ratesText(m.rates)} → ${ratesText(rates)}`)
    }
    const tiers: Tier[] = (upstream.cost.tiers ?? [])
      .filter((t: any) => t.tier?.type === "context" && t.tier.size > 0)
      .map((t: any) => ({ input_tokens_above: t.tier.size, rates: ratesFrom(t) }))
    const current = m.tiers ?? []
    if (tiers.length && tiersText(tiers) !== tiersText(current)) {
      setField(m, "tiers", tiersText(tiers), "rates")
      changes.push(`${name}: tiers ${current.length ? tiersText(current) : "none"} → ${tiersText(tiers)}`)
    } else if (!tiers.length && current.length) {
      notes.push(`${name} has long-context tiers models.dev no longer lists; check them by hand.`)
    }
  }
}

// Upstream models the catalog lacks, for a person to decide on, free, preview, and stealth models
// included. A model the catalog already lists a same or later version of (gpt-6-sol beside
// gpt-6.1-sol) is left out, as the catalog keeps each list to the current generation.
const listed = new Set(catalog.models.map((m) => `${m.provider}/${m.id}`))
const since = new Date(Date.now() - NEW_DAYS * 86_400_000).toISOString().slice(0, 10)
const family = (id: string) => id.replace(/\d+(?:\.\d+)*/g, "#")
const version = (id: string) => (id.match(/\d+/g) ?? []).map(Number)
const notOlder = (a: number[], b: number[]) => {
  for (let i = 0; i < Math.max(a.length, b.length); i++) if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0)
  return true
}
const superseded = (provider: string, id: string) =>
  catalog.models.some((m) => m.provider === provider && family(m.id) === family(id) && notOlder(version(m.id), version(id)))
const fresh: string[] = []
for (const [provider, source] of Object.entries(MODELS_DEV)) {
  for (const [id, e] of Object.entries<any>(modelsDev[source]?.models ?? {})) {
    if (listed.has(`${provider}/${id}`) || e.status === "deprecated" || (e.release_date ?? "") < since || !(e.limit?.output > 0)) continue
    if (superseded(provider, id)) continue
    const wire = wires[provider]?.get(id)
    if (provider in OPENCODE_DOCS && !wire) continue // not served on a wire Lorca speaks
    const levels = levelsFrom(e.reasoning_options)
    fresh.push(
      `${provider}/${id} — ${e.name}, released ${e.release_date}, ${e.limit.context} window, $${e.cost?.input ?? 0} / $${e.cost?.output ?? 0}${wire ? `, ${wire}` : ""}${levels ? `, levels ${levelsText(levels)}` : ""}`,
    )
  }
}
if (codex) {
  const visible = (codex.models ?? codex).filter((m: any) => m.visibility === "list").map((m: any) => m.slug)
  const chatgpt = catalog.models.filter((m) => m.provider === "chatgpt").map((m) => m.id)
  // Codex still lists the previous generation; only one at least as new as the catalog's counts.
  const generation = Math.max(...chatgpt.map((id) => version(id)[0] ?? 0))
  for (const slug of visible) {
    if (chatgpt.includes(slug) || (version(slug)[0] ?? 0) < generation || superseded("chatgpt", slug)) continue
    fresh.push(`chatgpt/${slug} — listed in Codex's models.json`)
  }
  for (const id of chatgpt) if (!visible.includes(id)) notes.push(`chatgpt/${id} is no longer listed in Codex's models.json.`)
}

if (changes.length) {
  // Later than both now and the time it had, which a hand edit may have set ahead.
  const time = Math.max(Date.now(), Date.parse(catalog.updated) + 1000)
  const updated = new Date(time).toISOString().replace(/\.\d+Z$/, "Z")
  text = text.replace(/"updated": "[^"]+"/, `"updated": "${updated}"`)
  JSON.parse(text)
  await Bun.write(CATALOG, text)
}

const section = (title: string, lines: string[]) => (lines.length ? `### ${title}\n\n${lines.map((l) => `- ${l}`).join("\n")}\n` : "")
const report = [
  section("Changed", changes),
  section(`Upstream models the catalog lacks (released in the last ${NEW_DAYS} days)`, fresh),
  section("To check", notes),
]
  .filter(Boolean)
  .join("\n")
console.log(report || "The catalog matches upstream.")
if (reportPath) await Bun.write(reportPath, report)
if (newPath) await Bun.write(newPath, fresh.map((line) => `- ${line}\n`).join(""))
