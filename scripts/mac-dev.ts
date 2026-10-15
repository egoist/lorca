// The deprecated AppKit app's dev loop (`bun run mac:dev`), kept while its code stays in macos/.
// `bun run dev` is the desktop app's.
import { watch } from "node:fs"
import { join, relative } from "node:path"
import {
  APP_NAME,
  CRATES_DIR,
  RESOURCES_DIR,
  MARKDOWN_SWIFT_DIR,
  PACKAGE_DIR,
  ROOT,
  SOURCES_DIR,
  appName,
  buildApp,
  bundlePath,
  color,
  log,
} from "./app.ts"
import { restartRelay, startRelay, stopRelay } from "./relay.ts"

const CONFIG = "debug" as const
const DEBOUNCE_MS = 80

let app: Bun.Subprocess | null = null
let building = false
let queued = false
let stopping = false

async function lorcaPids(): Promise<number[]> {
  const proc = Bun.spawn(["pgrep", "-x", APP_NAME], { stdout: "pipe", stderr: "pipe" })
  const text = await new Response(proc.stdout).text()
  await proc.exited
  const pids = text
    .split("\n")
    .map((line) => Number(line.trim()))
    .filter((pid) => Number.isInteger(pid) && pid > 0)
  // Only the development Mac bundle belongs to this loop. The production app and the phone
  // app may both have an executable named Lorca and stay running beside it.
  const mac: number[] = []
  const developmentExecutable = `${appName(CONFIG)}.app/Contents/MacOS/${APP_NAME}`
  for (const pid of pids) {
    const ps = Bun.spawn(["ps", "-o", "command=", "-p", String(pid)], { stdout: "pipe", stderr: "pipe" })
    const command = await new Response(ps.stdout).text()
    await ps.exited
    if (command.includes(developmentExecutable)) mac.push(pid)
  }
  return mac
}

function killPid(pid: number, signal: NodeJS.Signals) {
  try {
    process.kill(pid, signal)
  } catch {
    // Already gone.
  }
}

/** Quit every Lorca Dev process, not only the pid this script spawned last. */
async function stopApp() {
  const current = app
  app = null
  if (current?.pid) killPid(current.pid, "SIGTERM")

  let pids = await lorcaPids()
  for (const pid of pids) killPid(pid, "SIGTERM")

  for (let attempt = 0; attempt < 40; attempt++) {
    pids = await lorcaPids()
    if (pids.length === 0) {
      if (current) await Promise.race([current.exited, Bun.sleep(250)])
      return
    }
    if (attempt === 10) {
      log(color.yellow(`Lorca still running (${pids.join(", ")}) — sending SIGKILL`))
      for (const pid of pids) killPid(pid, "SIGKILL")
    }
    await Bun.sleep(50)
  }

  pids = await lorcaPids()
  if (pids.length > 0) {
    log(color.red(`could not stop Lorca pids ${pids.join(", ")}`))
  }
}

/** The terminal this loop writes to, so the app launched through LaunchServices can print
 * here too. Empty when stdout is not a terminal (piped, or a background task). */
async function ttyPath(): Promise<string> {
  const proc = Bun.spawn(["ps", "-o", "tty=", "-p", String(process.pid)], { stdout: "pipe", stderr: "pipe" })
  const name = (await new Response(proc.stdout).text()).trim()
  await proc.exited
  return name && name !== "??" ? `/dev/${name}` : ""
}

/** Launch the bundle through LaunchServices, not by spawning its executable. A binary spawned
 * from this script counts as the terminal's work: TCC charges microphone and speech requests
 * to the terminal app as the responsible process and reads the usage strings from *its*
 * Info.plist, and aborts the app when one is missing there (Kero has no speech string, so the
 * Dictate button crashed). Through `open` the app is its own responsible process and TCC reads
 * the bundle's own Info.plist. `open -W` exits when the app does; its environment is not
 * inherited, so LORCA_DEV goes through `--env`. */
async function startApp() {
  const bundle = bundlePath(CONFIG)
  const output = (await ttyPath()) || join(PACKAGE_DIR, ".build", "app.log")
  app = Bun.spawn(["open", "-n", "-W", "--stdout", output, "--stderr", output, "--env", "LORCA_DEV=1", bundle], {
    stdin: "ignore",
    stdout: "inherit",
    stderr: "inherit",
    onExit(_proc, exitCode, signal) {
      if (stopping || app === null) return
      app = null
      const how = signal ? `signal ${signal}` : `code ${exitCode}`
      log(color.yellow(`${appName(CONFIG)} exited (${how}) — press ${color.bold("r")} to relaunch`))
    },
  })
  const where = output.startsWith("/dev/") ? "" : ` · output in ${output}`
  log(`${color.green("running")} ${color.dim(`via open · CLI 127.0.0.1:4863${where}`)}`)
}

async function cycle(reason: string) {
  if (building) {
    queued = true
    return
  }
  building = true
  log(`${color.bold("building")} ${color.dim(reason)}`)

  await stopApp()
  const result = await buildApp(CONFIG)
  if (result.ok) {
    if (!stopping) await startApp()
    log(`${color.green("ready")} ${color.dim(`in ${Math.round(result.ms)}ms`)}`)
  } else {
    log(color.red("build failed — app not relaunched"))
  }

  building = false
  if (queued && !stopping) {
    queued = false
    await cycle("queued change")
  }
}

function watchSources() {
  let timer: ReturnType<typeof setTimeout> | null = null
  // One save often lands as a burst of events; each side remembers whether it was touched,
  // so a relay file and an app file saved together restart the relay and rebuild the app.
  let relayChanged = false
  let appChanged = ""
  // The build writes the markdown bindings into the Swift sources. They change only when the
  // markdown crate does, which is watched itself; counting them as a save would make every
  // build queue the next one.
  const generated = `${relative(SOURCES_DIR, MARKDOWN_SWIFT_DIR)}/`
  const onChange = (_event: string, filename: string | null) => {
    if (!filename) return
    if (![".swift", ".rs", "Cargo.toml", ".strings"].some((suffix) => filename.endsWith(suffix))) return
    if (filename.startsWith(generated)) return
    // The relay crate stands alone: its changes rebuild the relay, not the app.
    if (filename.startsWith("relay/")) relayChanged = true
    else appChanged = filename
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      const restart = relayChanged
      const rebuild = appChanged
      relayChanged = false
      appChanged = ""
      if (restart) void restartRelay()
      if (rebuild) void cycle(rebuild)
    }, DEBOUNCE_MS)
  }

  watch(SOURCES_DIR, { recursive: true }, onChange)
  watch(CRATES_DIR, { recursive: true }, onChange)
  // The string tables: the bundle step copies them, so a saved translation shows after a relaunch.
  watch(RESOURCES_DIR, { recursive: true }, onChange)
  watch(join(PACKAGE_DIR, "Package.swift"), () => void cycle("Package.swift"))
}

function watchKeys() {
  if (!process.stdin.isTTY) return
  process.stdin.setRawMode(true)
  process.stdin.resume()
  process.stdin.setEncoding("utf8")
  process.stdin.on("data", (key: string) => {
    if (key === "\u0003" || key === "q") void shutdown()
    else if (key === "r") void (async () => (await stopApp(), await startApp()))()
    else if (key === "b") void cycle("manual rebuild")
  })
}

async function shutdown() {
  if (stopping) return
  stopping = true
  await stopApp()
  await stopRelay(true)
  process.exit(0)
}

process.on("SIGINT", () => void shutdown())
process.on("SIGTERM", () => void shutdown())

console.log()
log(`${color.bold(appName(CONFIG))} — ${color.dim("r relaunch · b rebuild · q quit")}`)
await startRelay()
await cycle("initial build")
watchSources()
watchKeys()
