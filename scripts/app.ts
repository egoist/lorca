import { rm, mkdir, chmod } from "node:fs/promises"
import { join, resolve } from "node:path"

export const ROOT = resolve(import.meta.dir, "..")
export const PACKAGE_DIR = join(ROOT, "macos")
export const SOURCES_DIR = join(PACKAGE_DIR, "Sources")

export const CRATES_DIR = join(ROOT, "crates")
export const CLI_NAME = "tinybot"

export const APP_NAME = "Tinybot"
export const BUNDLE_ID = "dev.tinybot.app"
export const VERSION = "0.1.0"

export type Config = "debug" | "release"

export function bundlePath(config: Config) {
  return join(PACKAGE_DIR, ".build", "bundle", config, `${APP_NAME}.app`)
}

export function executablePath(config: Config) {
  return join(bundlePath(config), "Contents", "MacOS", APP_NAME)
}

export const color = {
  dim: (s: string) => `\x1b[2m${s}\x1b[0m`,
  bold: (s: string) => `\x1b[1m${s}\x1b[0m`,
  red: (s: string) => `\x1b[31m${s}\x1b[0m`,
  green: (s: string) => `\x1b[32m${s}\x1b[0m`,
  yellow: (s: string) => `\x1b[33m${s}\x1b[0m`,
  cyan: (s: string) => `\x1b[36m${s}\x1b[0m`,
}

export function log(message: string) {
  const time = new Date().toLocaleTimeString("en-US", { hour12: false })
  console.log(`${color.dim(time)} ${color.cyan("tinybot")} ${message}`)
}

function infoPlist() {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>${APP_NAME}</string>
	<key>CFBundleExecutable</key>
	<string>${APP_NAME}</string>
	<key>CFBundleIdentifier</key>
	<string>${BUNDLE_ID}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>${APP_NAME}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>${VERSION}</string>
	<key>CFBundleVersion</key>
	<string>${VERSION}</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.productivity</string>
	<key>LSMinimumSystemVersion</key>
	<string>14.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSPrincipalClass</key>
	<string>NSApplication</string>
	<key>NSSupportsAutomaticTermination</key>
	<false/>
	<key>NSSupportsSuddenTermination</key>
	<false/>
</dict>
</plist>
`
}

async function run(cmd: string[], opts: { cwd?: string; capture?: boolean } = {}) {
  const proc = Bun.spawn(cmd, {
    cwd: opts.cwd ?? PACKAGE_DIR,
    stdout: opts.capture ? "pipe" : "inherit",
    stderr: opts.capture ? "pipe" : "inherit",
  })
  const stdout = opts.capture ? await new Response(proc.stdout).text() : ""
  const exitCode = await proc.exited
  return { exitCode, stdout }
}

/** Compile the Rust CLI the app bundles and launches. */
export async function buildCLI(config: Config): Promise<{ ok: boolean; path: string }> {
  const args = ["build", "-q", "-p", CLI_NAME]
  if (config === "release") args.push("--release")
  const build = await run(["cargo", ...args], { cwd: ROOT })
  return { ok: build.exitCode === 0, path: join(ROOT, "target", config, CLI_NAME) }
}

/** Compile the SPM target and lay the product out as a launchable .app bundle with the CLI inside. */
export async function buildApp(config: Config): Promise<{ ok: boolean; ms: number }> {
  const started = performance.now()

  const cli = await buildCLI(config)
  if (!cli.ok) {
    return { ok: false, ms: performance.now() - started }
  }

  const build = await run(["swift", "build", "-c", config])
  if (build.exitCode !== 0) {
    return { ok: false, ms: performance.now() - started }
  }

  const binDir = await run(["swift", "build", "-c", config, "--show-bin-path"], { capture: true })
  const source = join(binDir.stdout.trim(), APP_NAME)

  const bundle = bundlePath(config)
  const macos = join(bundle, "Contents", "MacOS")
  await mkdir(macos, { recursive: true })
  await mkdir(join(bundle, "Contents", "Resources"), { recursive: true })
  await Bun.write(join(bundle, "Contents", "Info.plist"), infoPlist())
  await Bun.write(join(bundle, "Contents", "PkgInfo"), "APPL????")

  // Unlink before writing: macOS refuses to overwrite a running executable in place.
  const destination = join(macos, APP_NAME)
  await rm(destination, { force: true })
  await Bun.write(destination, Bun.file(source))
  await chmod(destination, 0o755)

  // The app launches this binary as `tinybot serve`. It lives under Resources/bin: on a
  // case-insensitive volume, MacOS/tinybot would be the same file as MacOS/Tinybot.
  const cliBinDir = join(bundle, "Contents", "Resources", "bin")
  await mkdir(cliBinDir, { recursive: true })
  const cliDestination = join(cliBinDir, CLI_NAME)
  await rm(cliDestination, { force: true })
  await Bun.write(cliDestination, Bun.file(cli.path))
  await chmod(cliDestination, 0o755)

  const sign = await run(
    ["codesign", "--force", "--sign", "-", "--identifier", BUNDLE_ID, bundle],
    { capture: true },
  )
  if (sign.exitCode !== 0) {
    return { ok: false, ms: performance.now() - started }
  }

  return { ok: true, ms: performance.now() - started }
}
