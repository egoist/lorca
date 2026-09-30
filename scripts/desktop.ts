// The Windows and Linux app in desktop/, built with MyGo.
//
//   bun run desktop                       Lorca Dev with live reload: builds the CLI for this
//                                         computer and runs `mygo dev`, whose app launches it
//                                         (LORCA_CLI) with LORCA_DEV=1, as the Mac dev loop does.
//   bun run desktop:build [platforms]     The release apps: the CLI for each platform into
//                                         desktop/resources/<goos>-<goarch>/bin, then
//                                         `mygo build -platform`. Platforms are MyGo's, comma
//                                         separated (linux/amd64,windows/amd64); the default is
//                                         this computer's, or Linux and Windows on x86-64 from a
//                                         Mac. The Linux CLIs are static (musl) and, like other
//                                         computers' CLIs, build with cargo-zigbuild.

import { copyFileSync, chmodSync, mkdirSync } from "node:fs"
import { join } from "node:path"
import { CLI_NAME, ROOT, buildCLI, color, log } from "./app.ts"

const DESKTOP = join(ROOT, "desktop")
const MYGO = join(DESKTOP, "node_modules", ".bin", process.platform === "win32" ? "mygo.exe" : "mygo")

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
  return await run([MYGO, "dev"], { cwd: DESKTOP, env: { LORCA_CLI: binary, LORCA_DEV: "1" } })
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
    const cli = await buildCLI("release")
    if (!cli.ok) return false
    built = `${cli.path}.exe`
  } else {
    log(`${color.bold("building")} ${color.dim(`the CLI for ${target}`)}`)
    if ((await run(["cargo", "zigbuild", "-q", "--release", "--locked", "-p", CLI_NAME, "--target", target])) !== 0) return false
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

async function build(platforms: string[]): Promise<number> {
  for (const platform of platforms) {
    if (!(await placeCLI(platform))) {
      log(color.red(`the CLI for ${platform} did not build`))
      return 1
    }
  }
  log(`${color.bold("building")} ${color.dim(`the app for ${platforms.join(", ")}`)}`)
  const status = await run([MYGO, "build", "-platform", platforms.join(",")], { cwd: DESKTOP })
  if (status === 0) log(`${color.green("built")} ${color.dim(join(DESKTOP, "build"))}`)
  return status
}

const [mode, list] = process.argv.slice(2)
if (mode === "build") {
  const host = hostPlatform()
  const platforms = list?.split(",").filter((platform) => platform !== "") ?? (host.startsWith("darwin") ? ["linux/amd64", "windows/amd64"] : [host])
  process.exit(await build(platforms))
}
process.exit(await dev())
