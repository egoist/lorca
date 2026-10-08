// The Windows and Linux app in desktop/, built with MyGo.
//
//   bun run desktop                       Lorca Dev with live reload: builds the CLI for this
//                                         computer and runs `go tool mygo dev`, whose app launches it
//                                         (LORCA_CLI) with LORCA_DEV=1, as the Mac dev loop does.
//   bun run desktop:build [platforms]     The release apps: the CLI for each platform into
//                                         desktop/resources/<goos>-<goarch>/bin, then
//                                         `go tool mygo build -platform`. Platforms are MyGo's, comma
//                                         separated (linux/amd64,windows/amd64); the default is
//                                         this computer's, or Linux and Windows on x86-64 from a
//                                         Mac. The Linux CLIs are static (musl) and, like other
//                                         computers' CLIs, build with cargo-zigbuild.
//   bun run release-desktop [platforms]   desktop:build signed with the update key and uploaded to
//                                         the draft release desktop-v<version> of egoist/lorca
//                                         (`mygo build -upload`), which the apps see once it is
//                                         published: docs/releasing-desktop.md.

import { copyFileSync, chmodSync, existsSync, mkdirSync, readFileSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { CLI_NAME, ROOT, buildCLI, color, log } from "./app.ts"
import { extractReleaseNotes } from "./changelog.ts"

const DESKTOP = join(ROOT, "desktop")
/** MyGo's command, the version desktop/go.mod pins as a tool. */
const MYGO = ["go", "tool", "mygo"]
/** `updates.github` and `updates.tagPrefix` of desktop/mygo.config.ts: the newest release with the
 * prefix is where installed apps look. */
const RELEASES_REPO = "egoist/lorca"
const TAG_PREFIX = "desktop-v"

/** The Rust target of the CLI each MyGo platform ships, static builds for Linux. */
const RUST_TARGETS: Record<string, string> = {
  "linux/amd64": "x86_64-unknown-linux-musl",
  "linux/arm64": "aarch64-unknown-linux-musl",
  "windows/amd64": "x86_64-pc-windows-gnu",
}

function hostPlatform(): string {
  const os = process.platform === "win32" ? "windows" : process.platform
  const arch = process.arch === "x64" ? "amd64" : process.arch
  return `${os}/${arch}`
}

async function run(command: string[], options: { cwd?: string; env?: Record<string, string> } = {}): Promise<number> {
  const child = Bun.spawn(command, {
    cwd: options.cwd ?? ROOT,
    env: { ...process.env, ...options.env },
    stdout: "inherit",
    stderr: "inherit",
    stdin: "inherit",
  })
  return await child.exited
}

async function dev(): Promise<number> {
  log(`${color.bold("building")} ${color.dim("the CLI (debug)")}`)
  const cli = await buildCLI("debug")
  if (!cli.ok) {
    log(color.red("the CLI did not build"))
    return 1
  }
  const binary = process.platform === "win32" ? `${cli.path}.exe` : cli.path
  log(`${color.bold("running")} ${color.dim("mygo dev")}`)
  return await run([...MYGO, "dev"], { cwd: DESKTOP, env: { LORCA_CLI: binary, LORCA_DEV: "1" } })
}

/** The CLI for `platform`, built for its Rust target and placed where the app finds it. */
async function placeCLI(platform: string): Promise<boolean> {
  const target = RUST_TARGETS[platform]
  if (!target) {
    log(color.red(`${platform} is not a platform the desktop app ships for`))
    return false
  }
  const [goos, goarch] = platform.split("/")
  const exe = goos === "windows" ? `${CLI_NAME}.exe` : CLI_NAME
  let built: string
  if (goos === "windows" && platform === hostPlatform()) {
    // Windows builds its own CLI with its own toolchain.
    log(`${color.bold("building")} ${color.dim("the CLI (release)")}`)
    const cli = await buildCLI("release")
    if (!cli.ok) return false
    built = `${cli.path}.exe`
  } else {
    log(`${color.bold("building")} ${color.dim(`the CLI for ${target}`)}`)
    if ((await run(["cargo", "zigbuild", "--release", "--locked", "-p", CLI_NAME, "--target", target])) !== 0) return false
    built = join(ROOT, "target", target, "release", exe)
  }
  const directory = join(DESKTOP, "resources", `${goos}-${goarch}`, "bin")
  mkdirSync(directory, { recursive: true })
  const placed = join(directory, exe)
  copyFileSync(built, placed)
  if (goos !== "windows") chmodSync(placed, 0o755)
  log(`${color.green("placed")} ${color.dim(placed)}`)
  return true
}

/** Where `mygo keygen` writes the update keys: MyGo's folder in the user's configuration directory. */
function keygenDirectory(): string {
  const home = homedir()
  const config =
    process.platform === "win32"
      ? (process.env.APPDATA ?? join(home, "AppData", "Roaming"))
      : process.platform === "darwin"
        ? join(home, "Library", "Application Support")
        : process.env.XDG_CONFIG_HOME || join(home, ".config")
  return join(config, "mygo", "update-keys")
}

/** What an upload needs besides the build: the update signing key, from MYGO_UPDATER_PRIVATE_KEY or
 * where `mygo keygen` put it, and the GitHub CLI with access to the releases repository (its
 * login, or GH_TOKEN). Null when something is missing. */
function releaseEnv(): Record<string, string> | null {
  const missing: string[] = []
  let key = process.env.MYGO_UPDATER_PRIVATE_KEY ?? ""
  if (key === "") {
    const file = join(keygenDirectory(), "mygo-update.key")
    if (existsSync(file)) key = readFileSync(file, "utf8").trim()
    else missing.push(`the update signing key: MYGO_UPDATER_PRIVATE_KEY, or ${file}`)
  }
  if (!Bun.which("gh")) missing.push("the GitHub CLI, gh")
  else if (Bun.spawnSync(["gh", "release", "list", "--repo", RELEASES_REPO, "--limit", "1"]).exitCode !== 0) {
    missing.push(`access to ${RELEASES_REPO}: gh auth login, or GH_TOKEN`)
  }
  for (const thing of missing) log(color.red(`missing ${thing}`))
  return missing.length === 0 ? { MYGO_UPDATER_PRIVATE_KEY: key } : null
}

/** Whether the release `tag` of the releases repository is a draft, published, or not there. */
function releaseState(tag: string): "draft" | "published" | "none" {
  const view = Bun.spawnSync(["gh", "release", "view", tag, "--repo", RELEASES_REPO, "--json", "isDraft", "--jq", ".isDraft"])
  if (view.exitCode !== 0) return "none"
  return view.stdout.toString().trim() === "true" ? "draft" : "published"
}

/** `version` in desktop/mygo.config.ts. */
async function desktopVersion(): Promise<string> {
  const config = (await import(join(DESKTOP, "mygo.config.ts"))).default as (env: { command: string }) => { version: string }
  return config({ command: "build" }).version
}

async function build(platforms: string[], options: { upload?: boolean } = {}): Promise<number> {
  let env: Record<string, string> = {}
  // The desktop app's own version, apart from the Mac app's.
  const version = await desktopVersion()
  if (options.upload) {
    const release = releaseEnv()
    if (!release) return 1
    env = release
    // mygo build reads the notes too, but only once the apps are built.
    if (!extractReleaseNotes(await Bun.file(join(DESKTOP, "CHANGELOG.md")).text(), version)) {
      log(color.red(`desktop/CHANGELOG.md has no "## [${version}]" section: add the notes of the update window`))
      return 1
    }
    if (releaseState(TAG_PREFIX + version) === "published" && process.env.FORCE !== "1") {
      log(color.red(`${TAG_PREFIX}${version} is already published: bump "version" in desktop/mygo.config.ts, or FORCE=1 to replace its files`))
      return 1
    }
  }
  for (const platform of platforms) {
    if (!(await placeCLI(platform))) {
      log(color.red(`the CLI for ${platform} did not build`))
      return 1
    }
  }
  log(`${color.bold("building")} ${color.dim(`the app for ${platforms.join(", ")}`)}`)
  const command = [...MYGO, "build", "-platform", platforms.join(","), ...(options.upload ? ["-upload"] : [])]
  const status = await run(command, { cwd: DESKTOP, env })
  if (status === 0 && options.upload) log(`${color.green("uploaded")} ${color.dim(`to the release ${TAG_PREFIX}${version} of ${RELEASES_REPO}`)}`)
  else if (status === 0) log(`${color.green("built")} ${color.dim(join(DESKTOP, "build"))}`)
  return status
}

const [mode, list] = process.argv.slice(2)
if (mode === "build" || mode === "release") {
  const host = hostPlatform()
  const platforms = list?.split(",").filter((platform) => platform !== "") ?? (host.startsWith("darwin") ? ["linux/amd64", "windows/amd64"] : [host])
  process.exit(await build(platforms, { upload: mode === "release" }))
}
process.exit(await dev())
