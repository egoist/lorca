// The phone app's dev loop: `bun run mobile:dev` for the booted simulator, `--phone` for the
// iPhone paired with this Mac, `--device <name or udid>` for any other.
//
// The installed app holds two things Metro cannot reload: the Rust core (crates/mobile over
// UniFFI, built into modules/lorca-core) and the native project (ios/, from app.json, the
// plugins, and the native modules in package.json). This script fingerprints the inputs of
// each, rebuilds what is stale, installs the app, starts Metro, and opens the app on it. A
// Rust save while it runs rebuilds the core and installs the app again.
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, watch, writeFileSync } from "node:fs"
import { networkInterfaces } from "node:os"
import { join, relative } from "node:path"
import { CRATES_DIR, ROOT, color, log } from "./app.ts"

const MOBILE = join(ROOT, "mobile")
const MODULE = join(MOBILE, "modules", "lorca-core")
const STAMPS = join(MOBILE, ".expo", "dev-stamps.json")
const METRO_PORT = 8081
const DEBOUNCE_MS = 500
// CocoaPods dies on a non-UTF-8 locale, and the CommandLineTools SDK breaks the pod install
// and the build with "unknown architecture" from tapi.
const NATIVE_ENV = { LANG: "en_US.UTF-8", LC_ALL: "en_US.UTF-8", DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer" }

/** `apps` is per device: an app installed on the simulator says nothing about the phone. */
type Stamps = { core?: string; project?: string; pods?: string; apps?: Record<string, string> }

let metro: Bun.Subprocess | null = null
let building = false
let queued = false
let stopping = false

function argument(name: string): string | undefined {
  const index = process.argv.indexOf(name)
  return index === -1 ? undefined : process.argv[index + 1]
}

async function run(cmd: string[], cwd: string, env: Record<string, string> = {}): Promise<boolean> {
  log(color.dim(`$ ${cmd.join(" ")}`))
  const proc = Bun.spawn(cmd, { cwd, env: { ...process.env, ...env }, stdin: "ignore", stdout: "inherit", stderr: "inherit" })
  return (await proc.exited) === 0
}

async function output(cmd: string[]): Promise<string> {
  const proc = Bun.spawn(cmd, { stdout: "pipe", stderr: "pipe" })
  const text = await new Response(proc.stdout).text()
  await proc.exited
  return text
}

// MARK: - Fingerprints

function files(path: string, keep: (path: string) => boolean, skip: string[] = []): string[] {
  if (!existsSync(path)) return []
  if (statSync(path).isFile()) return keep(path) ? [path] : []
  return readdirSync(path, { withFileTypes: true }).flatMap((entry) => {
    if (skip.includes(entry.name)) return []
    const child = join(path, entry.name)
    return entry.isDirectory() ? files(child, keep, skip) : keep(child) ? [child] : []
  })
}

/** Paths, sizes, and modification times: a save changes it, a checkout of the same tree too,
 * which costs one build that cargo and Xcode finish from their caches. */
function fingerprint(paths: string[]): string {
  const hasher = new Bun.CryptoHasher("sha256")
  for (const path of paths.sort()) {
    const stat = statSync(path)
    hasher.update(`${relative(ROOT, path)}:${stat.size}:${stat.mtimeMs}\n`)
  }
  return hasher.digest("hex")
}

/** What the core is built from: every crate the phone links, so all but the relay. */
function coreInputs(): string[] {
  const rust = (path: string) => path.endsWith(".rs") || path.endsWith("Cargo.toml")
  return [...files(CRATES_DIR, rust, ["relay", "target"]), join(ROOT, "Cargo.toml"), join(ROOT, "Cargo.lock"), join(MODULE, "build.ts")]
}

/** What `expo prebuild` and `pod install` read. */
function projectInputs(): string[] {
  const any = () => true
  return [join(MOBILE, "app.json"), join(MOBILE, "package.json"), ...files(join(MOBILE, "plugins"), any), ...files(join(MOBILE, "targets"), any)]
}

/** What `pod install` resolves, and where: Pods holds absolute paths into this checkout, so
 * a repo that moved needs them written again. */
function podsStamp(): string {
  return `${MOBILE}:${fingerprint([join(MOBILE, "package.json"), join(MOBILE, "bun.lock")])}`
}

/** Native sources compiled into the app, the core's build products among them. */
function appInputs(): string[] {
  const native = (path: string) => /\.(swift|h|m|mm|podspec|modulemap|a)$/.test(path) || path.endsWith("expo-module.config.json")
  return files(MODULE, native, ["android", "node_modules"])
}

function readStamps(): Stamps {
  try {
    return JSON.parse(readFileSync(STAMPS, "utf8"))
  } catch {
    return {}
  }
}

function writeStamps(stamps: Stamps) {
  mkdirSync(join(MOBILE, ".expo"), { recursive: true })
  writeFileSync(STAMPS, JSON.stringify(stamps, null, 2))
}

// MARK: - Device

/** The iPhone paired with this Mac, a connected one first. */
async function phone(): Promise<string> {
  const path = join(MOBILE, ".expo", "devices.json")
  mkdirSync(join(MOBILE, ".expo"), { recursive: true })
  await output(["xcrun", "devicectl", "list", "devices", "--json-output", path])
  type Listed = { hardwareProperties?: { udid?: string; reality?: string; platform?: string }; connectionProperties?: { tunnelState?: string }; deviceProperties?: { name?: string } }
  const devices = (JSON.parse(readFileSync(path, "utf8")).result.devices as Listed[]).filter((d) => d.hardwareProperties?.reality === "physical" && d.hardwareProperties.platform === "iOS")
  const found = devices.find((d) => d.connectionProperties?.tunnelState === "connected") ?? devices[0]
  if (!found?.hardwareProperties?.udid) {
    log(color.red("no iPhone is paired with this Mac — plug it in, unlock it, and trust this computer"))
    process.exit(1)
  }
  log(`${color.green("phone")} ${color.dim(`${found.deviceProperties?.name} (${found.connectionProperties?.tunnelState})`)}`)
  return found.hardwareProperties.udid
}

let chosenPhone: string | undefined

/** `--phone` or `--device`, else the booted simulator, else whichever one Expo picks. */
async function device(): Promise<{ id?: string; simulator: boolean }> {
  if (process.argv.includes("--phone")) return { id: (chosenPhone ??= await phone()), simulator: false }
  const asked = argument("--device")
  const listed = JSON.parse(await output(["xcrun", "simctl", "list", "devices", "-j"])) as { devices: Record<string, { udid: string; name: string; state: string }[]> }
  const simulators = Object.values(listed.devices).flat()
  if (asked) return { id: asked, simulator: simulators.some((s) => s.udid === asked || s.name === asked) }
  return { id: simulators.find((s) => s.state === "Booted")?.udid, simulator: true }
}

async function installed(udid: string | undefined): Promise<boolean> {
  if (!udid) return false
  const proc = Bun.spawn(["xcrun", "simctl", "get_app_container", udid, "app.lorca"], { stdout: "ignore", stderr: "ignore" })
  return (await proc.exited) === 0
}

// MARK: - Build

/** Rebuilds whatever is stale and installs the app. False when a step failed. */
async function build(reason: string): Promise<boolean> {
  const started = performance.now()
  const stamps = readStamps()
  const target = await device()

  const core = fingerprint(coreInputs())
  if (stamps.core !== core || !existsSync(join(MODULE, "ios", "LorcaCore.xcframework"))) {
    log(`${color.bold("core")} ${color.dim(reason)}`)
    if (!(await run(["bun", "run", "core", "ios"], MOBILE))) return false
    stamps.core = core
    writeStamps(stamps)
  }

  // `expo run:ios` leaves an existing ios/ alone, so a new plist key, plugin, or native
  // module reaches the app only through a clean prebuild.
  const project = fingerprint(projectInputs())
  const hasProject = existsSync(join(MOBILE, "ios", "Podfile"))
  if (!hasProject || (stamps.project !== undefined && stamps.project !== project)) {
    log(`${color.bold("prebuild")} ${color.dim(hasProject ? "app.json, package.json, or a plugin changed" : "no ios/ yet")}`)
    if (!(await run(["bunx", "expo", "prebuild", "--platform", "ios", "--clean", "--no-install"], MOBILE, NATIVE_ENV))) return false
    stamps.pods = undefined
  }
  stamps.project = project
  writeStamps(stamps)

  const pods = podsStamp()
  if (stamps.pods !== pods) {
    log(`${color.bold("pods")} ${color.dim(stamps.pods ? "dependencies changed, or the repo moved" : "not installed by this loop yet")}`)
    if (!(await run(["pod", "install"], join(MOBILE, "ios"), NATIVE_ENV))) return false
    stamps.pods = pods
    stamps.apps = {}
    writeStamps(stamps)
  }

  const app = fingerprint(appInputs())
  const key = target.id ?? "default"
  const present = target.simulator ? await installed(target.id) : stamps.apps?.[key] !== undefined
  if (stamps.apps?.[key] !== app || !present) {
    log(`${color.bold("app")} ${color.dim(present ? "native code changed" : "not installed yet")}`)
    const cmd = ["bunx", "expo", "run:ios", "--no-bundler", "--no-install", ...(target.id ? ["--device", target.id] : [])]
    // The first build after a pod install can miss codegen headers that the same command
    // finds on its second run.
    if (!(await run(cmd, MOBILE, NATIVE_ENV)) && !(await run(cmd, MOBILE, NATIVE_ENV))) return false
    stamps.apps = { ...stamps.apps, [key]: app }
    writeStamps(stamps)
  }

  log(`${color.green("ready")} ${color.dim(`in ${Math.round(performance.now() - started)}ms`)}`)
  return true
}

async function cycle(reason: string) {
  if (building) {
    queued = true
    return
  }
  building = true
  if (await build(reason)) await openApp()
  else log(color.red("build failed — the installed app is unchanged"))
  building = false
  if (queued && !stopping) {
    queued = false
    await cycle("queued change")
  }
}

// MARK: - Metro

async function metroAnswers(): Promise<boolean> {
  try {
    return (await fetch(`http://127.0.0.1:${METRO_PORT}/status`)).ok
  } catch {
    return false
  }
}

/** Metro keeps the terminal: its own keys (r reload, j debugger, m menu) work as usual. */
async function startMetro() {
  if (await metroAnswers()) {
    log(`${color.green("metro")} ${color.dim(`already listening on ${METRO_PORT}`)}`)
    return
  }
  metro = Bun.spawn(["bunx", "expo", "start", "--dev-client", "--port", String(METRO_PORT)], {
    cwd: MOBILE,
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
    onExit() {
      if (!stopping) void shutdown()
    },
  })
  for (let attempt = 0; attempt < 150 && !(await metroAnswers()); attempt++) await Bun.sleep(200)
}

/** This Mac's address on the network the phone shares with it. */
function lanAddress(): string | undefined {
  for (const [name, addresses] of Object.entries(networkInterfaces())) {
    if (!/^en\d/.test(name)) continue
    const found = addresses?.find((a) => a.family === "IPv4" && !a.internal)
    if (found) return found.address
  }
}

/** Points the dev client at Metro, so it skips its launcher screen: the simulator reaches
 * this Mac on 127.0.0.1, the phone on its LAN address. */
async function openApp() {
  const target = await device()
  const client = (host: string) => `exp+lorca://expo-development-client/?url=${encodeURIComponent(`http://${host}:${METRO_PORT}`)}`
  if (target.simulator) {
    await run(["xcrun", "simctl", "openurl", target.id ?? "booted", client("127.0.0.1")], ROOT)
    return
  }
  const host = lanAddress()
  if (!host || !target.id) return
  await run(["xcrun", "devicectl", "device", "process", "launch", "--terminate-existing", "--device", target.id, "--payload-url", client(host), "app.lorca"], ROOT)
}

function watchCore() {
  let timer: ReturnType<typeof setTimeout> | null = null
  watch(CRATES_DIR, { recursive: true }, (_event, filename) => {
    if (!filename || filename.startsWith("relay/")) return
    if (!filename.endsWith(".rs") && !filename.endsWith("Cargo.toml")) return
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => void cycle(filename), DEBOUNCE_MS)
  })
}

async function shutdown() {
  if (stopping) return
  stopping = true
  metro?.kill()
  if (metro) await Promise.race([metro.exited, Bun.sleep(2000)])
  process.exit(0)
}

process.on("SIGINT", () => void shutdown())
process.on("SIGTERM", () => void shutdown())

console.log()
log(`${color.bold("Lorca mobile")} dev — ${color.dim("a Rust save rebuilds the core and installs the app again")}`)
building = true
const ok = await build("initial build")
building = false
if (!ok) {
  log(color.red("build failed"))
  process.exit(1)
}
await startMetro()
await openApp()
watchCore()
