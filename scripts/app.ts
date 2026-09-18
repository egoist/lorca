import { rm, mkdir, chmod, readdir, readFile, rename, writeFile } from "node:fs/promises"
import { join, resolve } from "node:path"

export const ROOT = resolve(import.meta.dir, "..")
export const PACKAGE_DIR = join(ROOT, "macos")
export const SOURCES_DIR = join(PACKAGE_DIR, "Sources")

export const CRATES_DIR = join(ROOT, "crates")
export const CLI_NAME = "lorca"

export const APP_NAME = "Lorca"
export const BUNDLE_ID = "app.lorca"
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
  console.log(`${color.dim(time)} ${color.cyan("lorca")} ${message}`)
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
	<key>NSMicrophoneUsageDescription</key>
	<string>Lorca listens while you dictate a message.</string>
	<key>NSSpeechRecognitionUsageDescription</key>
	<string>Lorca turns what you say into the message text.</string>
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

export const MARKDOWN_CRATE = "lorca-markdown"
/** The generated Swift bindings the app compiles as its `LorcaMarkdown` target. */
export const MARKDOWN_SWIFT_DIR = join(SOURCES_DIR, "LorcaMarkdown")
/** The Rust static library and its C header, as the xcframework `Package.swift` links. */
export const MARKDOWN_XCFRAMEWORK = join(PACKAGE_DIR, "Libraries", "LorcaMarkdownFFI.xcframework")

/**
 * Compile the Markdown parser the app links: the static library, the Swift bindings, and the
 * xcframework that carries the library and its header to SwiftPM.
 */
export async function buildMarkdown(config: Config): Promise<{ ok: boolean }> {
  const args = ["build", "-q", "-p", MARKDOWN_CRATE]
  if (config === "release") args.push("--release")
  if ((await run(["cargo", ...args], { cwd: ROOT })).exitCode !== 0) return { ok: false }
  // The bindings come from the host dylib's metadata; the debug one is always current after
  // the build above, whichever configuration produced the static library.
  const dylib = join(ROOT, "target", config, "liblorca_markdown.dylib")
  const generated = join(ROOT, "target", "markdown-bindings")
  await rm(generated, { recursive: true, force: true })
  const bindgen = await run(
    ["cargo", "run", "-q", "-p", MARKDOWN_CRATE, "--features", "bindgen", "--bin", "uniffi-bindgen", "--", "generate", "--library", dylib, "--language", "swift", "--out-dir", generated],
    { cwd: ROOT },
  )
  if (bindgen.exitCode !== 0) return { ok: false }
  await mkdir(MARKDOWN_SWIFT_DIR, { recursive: true })
  const include = join(generated, "include")
  await mkdir(include, { recursive: true })
  for (const name of await readdir(generated)) {
    // Unchanged bindings stay as they are on disk, so the Swift target is not recompiled.
    if (name.endsWith(".swift")) {
      const fresh = await readFile(join(generated, name), "utf8")
      const current = await readFile(join(MARKDOWN_SWIFT_DIR, name), "utf8").catch(() => null)
      if (fresh !== current) await writeFile(join(MARKDOWN_SWIFT_DIR, name), fresh)
    }
    if (name.endsWith("FFI.h")) await rename(join(generated, name), join(include, name))
    if (name.endsWith("FFI.modulemap")) {
      await writeFile(join(include, "module.modulemap"), await readFile(join(generated, name)))
      await rm(join(generated, name))
    }
  }
  await rm(MARKDOWN_XCFRAMEWORK, { recursive: true, force: true })
  const framework = await run(
    ["xcodebuild", "-create-xcframework", "-library", join(ROOT, "target", config, "liblorca_markdown.a"), "-headers", include, "-output", MARKDOWN_XCFRAMEWORK],
    { cwd: ROOT, capture: true },
  )
  return { ok: framework.exitCode === 0 }
}

/** AppKit picks the app's look from the SDK version in the binary's LC_BUILD_VERSION. The Swift
 * Build engine writes the deployment target there (`sdk 14.0`), and AppKit then runs the app in
 * its pre-macOS 26 look: a flat sidebar, an opaque titlebar strip with a fixed separator, no
 * scroll-edge effect under the titlebar. Restamp with the SDK the app was compiled against. */
export async function stampSDK(binary: string): Promise<boolean> {
  const build = await run(["vtool", "-show-build", binary], { capture: true })
  const minos = build.stdout.match(/minos (\S+)/)?.[1]
  const stamped = build.stdout.match(/sdk (\S+)/)?.[1]
  const sdk = (await run(["xcrun", "--sdk", "macosx", "--show-sdk-version"], { capture: true })).stdout.trim()
  if (build.exitCode !== 0 || !minos || !sdk) return false
  if (stamped === sdk) return true
  const restamped = `${binary}.restamped`
  const vtool = await run(
    ["vtool", "-set-build-version", "macos", minos, sdk, "-replace", "-output", restamped, binary],
    { capture: true },
  )
  if (vtool.exitCode !== 0) return false
  await rename(restamped, binary)
  return true
}

/** Compile the SPM target and lay the product out as a launchable .app bundle with the CLI inside. */
export async function buildApp(config: Config): Promise<{ ok: boolean; ms: number }> {
  const started = performance.now()

  const cli = await buildCLI(config)
  if (!cli.ok) {
    return { ok: false, ms: performance.now() - started }
  }
  if (!(await buildMarkdown(config)).ok) {
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
  if (!(await stampSDK(destination))) {
    return { ok: false, ms: performance.now() - started }
  }
  await chmod(destination, 0o755)

  // The app launches this binary as `lorca serve`. It lives under Resources/bin: on a
  // case-insensitive volume, MacOS/lorca would be the same file as MacOS/Lorca.
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
