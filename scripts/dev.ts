// `bun run dev`: Lorca Dev, the desktop app in desktop/, with live reload, beside a local relay.
//
// The CLI builds first, into target/debug, and runs from a copy in temp/dev-cli, which the app
// launches (LORCA_CLI) with LORCA_DEV=1: `go tool mygo dev` in desktop/ rebuilds and relaunches
// the app on a Go save. A Rust save rebuilds the CLI, stops the CLI the app runs, and puts the
// new one in its place; the app starts it again on its own, as it does any CLI that exits. A
// copy, since a running program can't be overwritten on Windows. A relay save rebuilds the relay.
import { copyFileSync, chmodSync, mkdirSync, watch } from "node:fs"
import { join } from "node:path"
import { CLI_NAME, CRATES_DIR, ROOT, buildCLI, color, log } from "./app.ts"
import { restartRelay, startRelay, stopRelay } from "./relay.ts"

const DESKTOP = join(ROOT, "desktop")
const DEBOUNCE_MS = 80
const WINDOWS = process.platform === "win32"
const EXE = WINDOWS ? `${CLI_NAME}.exe` : CLI_NAME
/** The CLI the app runs, a copy of the one cargo builds. */
const RUN_DIR = join(ROOT, "temp", "dev-cli")
const RUN_PATH = join(RUN_DIR, EXE)

let app: Bun.Subprocess | null = null
let building = false
let queued = false
let stopping = false

/** Puts the CLI cargo built where the app runs it. */
function placeCLI(built: string) {
  mkdirSync(RUN_DIR, { recursive: true })
  copyFileSync(WINDOWS ? `${built}.exe` : built, RUN_PATH)
  if (!WINDOWS) chmodSync(RUN_PATH, 0o755)
}

/** The CLIs running from the copy: the one the app started, and any an earlier loop left. */
async function runningCLIs(): Promise<number[]> {
  if (WINDOWS) {
    const proc = Bun.spawn(["powershell", "-NoProfile", "-Command", `Get-Process | Where-Object { $_.Path -eq '${RUN_PATH}' } | ForEach-Object { $_.Id }`], { stdout: "pipe", stderr: "ignore" })
    const text = await new Response(proc.stdout).text()
    await proc.exited
    return text.split(/\s+/).map(Number).filter((pid) => Number.isInteger(pid) && pid > 0)
  }
  const proc = Bun.spawn(["ps", "-axo", "pid=,command="], { stdout: "pipe", stderr: "ignore" })
  const text = await new Response(proc.stdout).text()
  await proc.exited
  return text
    .split("\n")
    .map((line) => line.trim().match(/^(\d+)\s+(.*)$/))
    .filter((match): match is RegExpMatchArray => match !== null && match[2].startsWith(`${RUN_PATH} `))
    .map((match) => Number(match[1]))
}

/** Stops the CLI the app runs, then puts the new build in its place before the app starts it
 * again, a second later. */
async function swapCLI(built: string) {
  const pids = await runningCLIs()
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM")
    } catch {
      // Already gone.
    }
  }
  for (let attempt = 0; attempt < 40 && (await runningCLIs()).length > 0; attempt++) await Bun.sleep(25)
  placeCLI(built)
  if (pids.length > 0) log(`${color.green("CLI")} ${color.dim("replaced · the app starts it again")}`)
}

async function rebuildCLI(reason: string) {
  if (building) {
    queued = true
    return
  }
  building = true
  log(`${color.bold("building")} ${color.dim(`the CLI · ${reason}`)}`)
  const started = performance.now()
  const cli = await buildCLI("debug")
  if (cli.ok && !stopping) {
    await swapCLI(cli.path)
    log(`${color.green("ready")} ${color.dim(`in ${Math.round(performance.now() - started)}ms`)}`)
  } else if (!cli.ok) {
    log(color.red("the CLI did not build — the app keeps the one it runs"))
  }
  building = false
  if (queued && !stopping) {
    queued = false
    await rebuildCLI("queued change")
  }
}

function watchCrates() {
  let timer: ReturnType<typeof setTimeout> | null = null
  // One save often lands as a burst of events; each side remembers whether it was touched.
  let relayChanged = false
  let cliChanged = ""
  watch(CRATES_DIR, { recursive: true }, (_event, filename) => {
    if (!filename) return
    if (![".rs", "Cargo.toml", ".json"].some((suffix) => filename.endsWith(suffix))) return
    // The relay crate stands alone: its changes rebuild the relay, not the CLI.
    if (filename.startsWith("relay/")) relayChanged = true
    else cliChanged = filename
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      const restart = relayChanged
      const rebuild = cliChanged
      relayChanged = false
      cliChanged = ""
      if (restart) void restartRelay()
      if (rebuild) void rebuildCLI(rebuild)
    }, DEBOUNCE_MS)
  })
}

/** `go tool mygo dev`, which watches the Go sources and relaunches the app itself. */
function startApp() {
  app = Bun.spawn(["go", "tool", "mygo", "dev"], {
    cwd: DESKTOP,
    env: { ...process.env, LORCA_CLI: RUN_PATH, LORCA_DEV: "1" },
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
    onExit(_proc, exitCode, signal) {
      app = null
      if (stopping) return
      log(color.yellow(`mygo dev exited (${signal ? `signal ${signal}` : `code ${exitCode}`})`))
      void shutdown(exitCode ?? 1)
    },
  })
  log(`${color.green("running")} ${color.dim("mygo dev · CLI 127.0.0.1:4863")}`)
}

async function shutdown(code = 0) {
  if (stopping) return
  stopping = true
  if (app) {
    app.kill()
    await Promise.race([app.exited, Bun.sleep(3000)])
  }
  await stopRelay(true)
  process.exit(code)
}

process.on("SIGINT", () => void shutdown())
process.on("SIGTERM", () => void shutdown())

console.log()
log(`${color.bold("Lorca Dev")} — ${color.dim("Ctrl+C quits")}`)
await startRelay()
log(`${color.bold("building")} ${color.dim("the CLI (debug)")}`)
const cli = await buildCLI("debug")
if (!cli.ok) {
  log(color.red("the CLI did not build"))
  await shutdown(1)
}
await swapCLI(cli.path)
startApp()
watchCrates()
