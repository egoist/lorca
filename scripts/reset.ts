// Reset Lorca on this computer: stop the app and CLI, wipe identity, keys, credentials, chats,
// app preferences, and logs. Use --dev for Lorca Dev, --build to drop build output too,
// --relay for the local relay database, and -y to skip the confirmation. The app is the desktop
// app (desktop/), whose data MyGo keeps in the user's configuration folder under the app's name;
// the deprecated Mac app's preferences go too.
import { rm } from "node:fs/promises"
import { existsSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { APP_NAME, BUNDLE_ID, DEBUG_APP_NAME, DEBUG_BUNDLE_ID, PACKAGE_DIR, ROOT, color, log } from "./app.ts"

const args = process.argv.slice(2)
const wipeBuild = args.includes("--build")
const wipeRelay = args.includes("--relay")
const yes = args.includes("-y") || args.includes("--yes")
const development = args.includes("--dev")

const appName = development ? DEBUG_APP_NAME : APP_NAME
const bundleID = development ? DEBUG_BUNDLE_ID : BUNDLE_ID
const port = development ? 4863 : 4862
const home = process.env.LORCA_HOME ?? join(homedir(), development ? ".lorca-dev" : ".lorca")
const mac = process.platform === "darwin"
/** The user's configuration folder, as Go's os.UserConfigDir answers, which MyGo keeps an app's data in. */
const configDir = mac
  ? join(homedir(), "Library", "Application Support")
  : process.platform === "win32"
    ? (process.env.APPDATA ?? join(homedir(), "AppData", "Roaming"))
    : process.env.XDG_CONFIG_HOME || join(homedir(), ".config")
const appData = join(configDir, appName)
// Elsewhere MyGo keeps the logs in the app's data folder.
const logs = join(homedir(), "Library", "Logs", appName)

const targets: { path: string; what: string; on: boolean }[] = [
  { path: home, what: "identity, keys, credentials, chats", on: true },
  { path: appData, what: "app preferences and window state", on: true },
  { path: logs, what: "app and CLI logs", on: mac },
  { path: join(ROOT, "temp", "lorca-relay.db"), what: "local relay database", on: wipeRelay },
  { path: join(ROOT, "temp", "lorca-relay.db-wal"), what: "local relay database", on: wipeRelay },
  { path: join(ROOT, "temp", "lorca-relay.db-shm"), what: "local relay database", on: wipeRelay },
  { path: join(ROOT, "temp", "lorca-relay.files"), what: "local relay attachments", on: wipeRelay },
  { path: join(ROOT, "target"), what: "Rust build output", on: wipeBuild },
  { path: join(ROOT, "temp", "dev-cli"), what: "the dev loop's CLI", on: wipeBuild },
  { path: join(ROOT, "desktop", ".mygo"), what: "desktop app build output", on: wipeBuild },
  { path: join(ROOT, "desktop", "build"), what: "desktop app release builds", on: wipeBuild },
  { path: join(PACKAGE_DIR, ".build"), what: "Mac app build output", on: wipeBuild },
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
  // The app on macOS (the desktop app's bundle and the Mac app's), the dev loop's on Linux, its
  // CLI, and the local relay.
  const patterns = [`${appName}.app/Contents/MacOS/`, `desktop/.mygo/dev/.*/${appName}`, `lorca serve --port ${port}`, "lorca-relay"]
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
  process.stdout.write(`${color.yellow(`Reset ${appName}?`)} This deletes the identity on this computer; without the backup phrase it is gone. [y/N] `)
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
if (mac) console.log(`  ${color.red("✘")} defaults ${bundleID} ${color.dim("the Mac app's preferences")}`)
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

if (mac) {
  const defaults = Bun.spawn(["defaults", "delete", bundleID], { stdout: "ignore", stderr: "ignore" })
  await defaults.exited
  log(`cleared preferences ${color.dim(bundleID)}`)
}

log(color.green("done") + color.dim(" — next launch starts at onboarding"))
