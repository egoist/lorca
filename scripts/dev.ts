import { watch } from "node:fs"
import { join } from "node:path"
import {
  APP_NAME,
  CRATES_DIR,
  PACKAGE_DIR,
  ROOT,
  SOURCES_DIR,
  buildApp,
  color,
  executablePath,
  log,
} from "./app.ts"

const CONFIG = "debug" as const
const DEBOUNCE_MS = 80

let app: Bun.Subprocess | null = null
let relay: Bun.Subprocess | null = null
let building = false
let queued = false
let stopping = false

async function tinybotPids(): Promise<number[]> {
  const proc = Bun.spawn(["pgrep", "-x", APP_NAME], { stdout: "pipe", stderr: "pipe" })
  const text = await new Response(proc.stdout).text()
  await proc.exited
  return text
    .split("\n")
    .map((line) => Number(line.trim()))
    .filter((pid) => Number.isInteger(pid) && pid > 0)
}

function killPid(pid: number, signal: NodeJS.Signals) {
  try {
    process.kill(pid, signal)
  } catch {
    // Already gone.
  }
}

/** Quit every Tinybot process, not only the pid this script spawned last. */
async function stopApp() {
  const current = app
  app = null
  if (current?.pid) killPid(current.pid, "SIGTERM")

  let pids = await tinybotPids()
  for (const pid of pids) killPid(pid, "SIGTERM")

  for (let attempt = 0; attempt < 40; attempt++) {
    pids = await tinybotPids()
    if (pids.length === 0) {
      if (current) await Promise.race([current.exited, Bun.sleep(250)])
      return
    }
    if (attempt === 10) {
      log(color.yellow(`Tinybot still running (${pids.join(", ")}) — sending SIGKILL`))
      for (const pid of pids) killPid(pid, "SIGKILL")
    }
    await Bun.sleep(50)
  }

  pids = await tinybotPids()
  if (pids.length > 0) {
    log(color.red(`could not stop Tinybot pids ${pids.join(", ")}`))
  }
}

const RELAY_PORT = 8787

async function relayAnswers(): Promise<boolean> {
  try {
    return (await fetch(`http://127.0.0.1:${RELAY_PORT}/v1/health`)).ok
  } catch {
    return false
  }
}

/** A local relay on every interface, so a phone on this network can pair through it. The dev
 * app (TINYBOT_DEV=1) defaults its relay URL to this Mac's LAN IP on this port. */
async function startRelay() {
  if (relay) return
  if (await relayAnswers()) {
    log(`${color.green("relay")} ${color.dim(`already listening on ${RELAY_PORT}`)}`)
    return
  }
  relay = Bun.spawn(
    ["cargo", "run", "-q", "-p", "tinybot-relay", "--", "--bind", `0.0.0.0:${RELAY_PORT}`, "--db", join(ROOT, "target", "tinybot-relay.db")],
    {
      cwd: ROOT,
      stdin: "ignore",
      stdout: "inherit",
      stderr: "inherit",
      env: { ...process.env },
      onExit(_proc, exitCode, signal) {
        if (stopping || relay === null) return
        relay = null
        log(color.yellow(`relay exited (${signal ? `signal ${signal}` : `code ${exitCode}`})`))
      },
    },
  )
  log(`${color.green("relay")} ${color.dim(`0.0.0.0:${RELAY_PORT} · pairing codes carry this Mac's LAN IP`)}`)
}

function stopRelay() {
  const current = relay
  relay = null
  current?.kill()
}

function startApp() {
  const bin = executablePath(CONFIG)
  app = Bun.spawn([bin], {
    stdin: "ignore",
    stdout: "inherit",
    stderr: "inherit",
    env: { ...process.env, TINYBOT_DEV: "1" },
    onExit(_proc, exitCode, signal) {
      if (stopping || app === null) return
      app = null
      const how = signal ? `signal ${signal}` : `code ${exitCode}`
      log(color.yellow(`${APP_NAME} exited (${how}) — press ${color.bold("r")} to relaunch`))
    },
  })
  log(`${color.green("running")} ${color.dim(`pid ${app.pid}`)}`)
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
    if (!stopping) startApp()
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
  const onChange = (_event: string, filename: string | null) => {
    if (!filename) return
    if (!filename.endsWith(".swift") && !filename.endsWith(".rs") && !filename.endsWith("Cargo.toml")) return
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      if (filename.startsWith("relay/")) {
        stopRelay()
        void startRelay()
      }
      void cycle(filename)
    }, DEBOUNCE_MS)
  }

  watch(SOURCES_DIR, { recursive: true }, onChange)
  watch(CRATES_DIR, { recursive: true }, onChange)
  watch(join(PACKAGE_DIR, "Package.swift"), () => void cycle("Package.swift"))
}

function watchKeys() {
  if (!process.stdin.isTTY) return
  process.stdin.setRawMode(true)
  process.stdin.resume()
  process.stdin.setEncoding("utf8")
  process.stdin.on("data", (key: string) => {
    if (key === "\u0003" || key === "q") void shutdown()
    else if (key === "r") void (async () => (await stopApp(), startApp()))()
    else if (key === "b") void cycle("manual rebuild")
  })
}

async function shutdown() {
  if (stopping) return
  stopping = true
  await stopApp()
  stopRelay()
  process.exit(0)
}

process.on("SIGINT", () => void shutdown())
process.on("SIGTERM", () => void shutdown())

console.log()
log(`${color.bold(APP_NAME)} dev — ${color.dim("r relaunch · b rebuild · q quit")}`)
await startRelay()
await cycle("initial build")
watchSources()
watchKeys()
