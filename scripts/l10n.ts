// Checks the apps' string tables against the sources: every `L("…")` in the Mac app and every
// `t("…")` in the phone app should have a Chinese entry, and a format key and its translation
// should take the same values. `bun run l10n` lists what is missing, unused, or mismatched;
// `--merge <fragment.json>…` adds `{ "English": "中文" }` files to the tables first.

import { Glob } from "bun"
import { join } from "node:path"

const ROOT = join(import.meta.dir, "..")
const MAC_TABLE = join(ROOT, "macos/Resources/zh-Hans.lproj/Localizable.strings")
const PHONE_TABLE = join(ROOT, "mobile/src/i18n/zh.ts")

type Table = Map<string, string>

/// A Swift or TypeScript string literal's text, for the escapes the sources use.
function unescape(literal: string) {
  return literal.replace(/\\(u\{([0-9a-fA-F]+)\}|u([0-9a-fA-F]{4})|.)/g, (_, all, braced, plain) => {
    if (braced ?? plain) return String.fromCodePoint(parseInt(braced ?? plain, 16))
    return ({ n: "\n", t: "\t", r: "\r" } as Record<string, string>)[all] ?? all
  })
}

function escapeFor(text: string) {
  return text.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n").replace(/\t/g, "\\t")
}

async function keysIn(dir: string, pattern: string, call: RegExp, skip: (path: string) => boolean) {
  const keys = new Map<string, string>()
  for await (const path of new Glob(pattern).scan({ cwd: dir })) {
    if (skip(path)) continue
    const source = await Bun.file(join(dir, path)).text()
    for (const match of source.matchAll(call)) {
      // `L("Pairing", context: "device state")` is the key `Pairing|device state`.
      if (match[1] === undefined) keys.set(unescape(match[2]), path) // a single-quoted t('…')
      else keys.set(match[2] ? `${unescape(match[1])}|${unescape(match[2])}` : unescape(match[1]), path)
    }
  }
  return keys
}

async function readMacTable(): Promise<Table> {
  const table: Table = new Map()
  const file = Bun.file(MAC_TABLE)
  if (!(await file.exists())) return table
  for (const match of (await file.text()).matchAll(/^"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)";/gm)) {
    table.set(unescape(match[1]), unescape(match[2]))
  }
  return table
}

async function readPhoneTable(): Promise<Table> {
  const table: Table = new Map()
  const file = Bun.file(PHONE_TABLE)
  if (!(await file.exists())) return table
  for (const match of (await file.text()).matchAll(/^\s*"((?:[^"\\]|\\.)*)":\s*"((?:[^"\\]|\\.)*)",?$/gm)) {
    table.set(unescape(match[1]), unescape(match[2]))
  }
  return table
}

function sorted(table: Table) {
  return [...table].sort(([a], [b]) => a.localeCompare(b, "en"))
}

async function writeMacTable(table: Table) {
  const lines = sorted(table).map(([key, value]) => `"${escapeFor(key)}" = "${escapeFor(value)}";`)
  await Bun.write(MAC_TABLE, `/* Simplified Chinese. The key is the English text passed to L(); \`bun run l10n\` checks this table. */\n\n${lines.join("\n")}\n`)
}

async function writePhoneTable(table: Table) {
  const lines = sorted(table).map(([key, value]) => `  "${escapeFor(key)}": "${escapeFor(value)}",`)
  await Bun.write(PHONE_TABLE, `// Simplified Chinese. The key is the English text passed to t(); \`bun run l10n\` checks this table.\n\nexport const zh: Record<string, string> = {\n${lines.join("\n")}\n};\n`)
}

/// The values a sentence takes: `%@`/`%d` (positions ignored) or `{name}`.
function slots(text: string) {
  const printf = [...text.replace(/%%/g, "").matchAll(/%(?:\d+\$)?([@dfs])/g)].map((m) => m[1]).sort()
  const named = [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort()
  return [...printf, ...named].join(",")
}

function report(name: string, used: Map<string, string>, table: Table) {
  let problems = 0
  for (const [key, path] of used) {
    const value = table.get(key)
    if (value === undefined) {
      console.log(`${name}: no translation for ${JSON.stringify(key)} (${path})`)
      problems++
    } else if (slots(key) !== slots(value)) {
      console.log(`${name}: values differ in ${JSON.stringify(key)} → ${JSON.stringify(value)}`)
      problems++
    }
  }
  for (const key of table.keys()) {
    if (!used.has(key)) {
      console.log(`${name}: unused ${JSON.stringify(key)}`)
      problems++
    }
  }
  console.log(`${name}: ${used.size} strings, ${table.size} translated, ${problems} problems`)
  return problems
}

const macKeys = await keysIn(join(ROOT, "macos/Sources/Lorca"), "**/*.swift", /\bL\(\s*"((?:[^"\\]|\\.)*)"(?:,\s*context:\s*"((?:[^"\\]|\\.)*)")?/g, () => false)
const phoneKeys = await keysIn(join(ROOT, "mobile"), "{app,src}/**/*.{ts,tsx}", /\bt\(\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)')/g, (path) => path.includes("i18n/") || path.endsWith(".test.ts"))

const mac = await readMacTable()
const phone = await readPhoneTable()

const mergeAt = process.argv.indexOf("--merge")
if (mergeAt !== -1) {
  for (const path of process.argv.slice(mergeAt + 1)) {
    const fragment = (await Bun.file(path).json()) as Record<string, string>
    for (const [key, value] of Object.entries(fragment)) {
      for (const [used, table, name] of [[macKeys, mac, "mac"], [phoneKeys, phone, "phone"]] as const) {
        if (!used.has(key)) continue
        const existing = table.get(key)
        if (existing !== undefined && existing !== value) console.log(`${name}: kept ${JSON.stringify(existing)} over ${JSON.stringify(value)} for ${JSON.stringify(key)}`)
        else table.set(key, value)
      }
    }
  }
  await writeMacTable(mac)
  await writePhoneTable(phone)
}

const problems = report("mac", macKeys, mac) + report("phone", phoneKeys, phone)
process.exit(problems === 0 ? 0 : 1)
