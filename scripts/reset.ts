// Reset Tinybot on this Mac: stop the app and CLI, wipe identity, keys, credentials, chats,
// app preferences, and logs. Use --build to drop build output too, --relay for the local relay
// database, -y to skip the confirmation.
import { rm } from "node:fs/promises"
import { existsSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { APP_NAME, BUNDLE_ID, PACKAGE_DIR, ROOT, color, log } from "./app.ts"

const args = process.argv.slice(2)
const wipeBuild = args.includes("--build")
const wipeRelay = args.includes("--relay")
const yes = args.includes("-y") || args.includes("--yes")

const home = process.env.TINYBOT_HOME ?? join(homedir(), ".tinybot")
const logs = join(homedir(), "Library", "Logs", APP_NAME)

const targets: { path: string; what: string; on: boolean }[] = [
  { path: home, what: "identity, keys, credentials, chats", on: true },
  { path: logs, what: "CLI logs", on: true },
  { path: join(ROOT, "target", "tinybot-relay.db"), what: "local relay database", on: wipeRelay },
  { path: join(ROOT, "target", "tinybot-relay.db-wal"), what: "local relay database", on: wipeRelay },
  { path: join(ROOT, "target", "tinybot-relay.db-shm"), what: "local relay database", on: wipeRelay },
  { path: join(ROOT, "target", "tinybot-relay.files"), what: "local relay attachments", on: wipeRelay },
  { path: join(ROOT, "target"), what: "Rust build output", on: wipeBuild },
  { path: join(PACKAGE_DIR, ".build"), what: "Swift build output", on: wipeBuild },
]

async function pids(pattern: string): Promise<number[]> {
  const proc = Bun.spawn(["pgrep", "-f", pattern], { stdout: "pipe", stderr: "pipe" })
  const text = await new Response(proc.stdout).text()
  await proc.exited
  return text
    .split("\n")
    .map((line) => Number(line.trim()))
    .filter((pid) => Number.isInteger(pid) && pid > 0 && pid !== process.pid)
}

async function stopProcesses() {
  const patterns = [`${APP_NAME}.app/Contents/MacOS/${APP_NAME}`, "tinybot serve", "tinybot-relay"]
  let found: number[] = []
  for (const pattern of patterns) found = found.concat(await pids(pattern))
  found = [...new Set(found)]
  if (found.length === 0) {
    log(`${color.dim("no app, CLI, or relay running")}`)
    return
  }
  for (const pid of found) {
    try { process.kill(pid, "SIGTERM") } catch {}
  }
  await Bun.sleep(600)
  for (const pid of found) {
    try { process.kill(pid, 0); process.kill(pid, "SIGKILL") } catch {}
  }
  log(`stopped ${found.length} process${found.length === 1 ? "" : "es"} ${color.dim(found.join(", "))}`)
}

async function confirm(): Promise<boolean> {
  if (yes || !process.stdin.isTTY) return yes
  process.stdout.write(`${color.yellow("Reset Tinybot?")} This deletes the identity on this Mac; without the backup phrase it is gone. [y/N] `)
  for await (const chunk of Bun.stdin.stream()) {
    const answer = new TextDecoder().decode(chunk).trim().toLowerCase()
    return answer === "y" || answer === "yes"
  }
  return false
}

console.log()
log(`${color.bold("reset")} ${color.dim(home)}`)
for (const target of targets.filter((t) => t.on && existsSync(t.path))) {
  console.log(`  ${color.red("✘")} ${target.path} ${color.dim(target.what)}`)
}
console.log(`  ${color.red("✘")} defaults ${BUNDLE_ID} ${color.dim("app preferences")}`)
console.log()

if (!(await confirm())) {
  log(color.dim("nothing changed (pass -y to skip the prompt)"))
  process.exit(1)
}

await stopProcesses()

for (const target of targets.filter((t) => t.on)) {
  if (!existsSync(target.path)) continue
  await rm(target.path, { recursive: true, force: true })
  log(`removed ${target.path}`)
}

const defaults = Bun.spawn(["defaults", "delete", BUNDLE_ID], { stdout: "ignore", stderr: "ignore" })
await defaults.exited
log(`cleared preferences ${color.dim(BUNDLE_ID)}`)

log(color.green("done") + color.dim(" — next launch starts at onboarding"))
