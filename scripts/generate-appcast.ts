// Sign a directory of update archives and (re)write Sparkle's appcast.xml there.
//
//   bun scripts/generate-appcast.ts <updates-dir>
//
// <updates-dir> holds Lorca-<version>.zip archives, the older ones included so Sparkle can build
// binary deltas between them. The private EdDSA key comes from the login keychain
// (docs/releasing-mac.md).
import { existsSync, readdirSync } from "node:fs"
import { join } from "node:path"
import { color, log, RELEASES_URL, SPARKLE_TOOLS } from "./app.ts"

/** `SPARKLE_BIN`, then the copy SwiftPM unpacked beside the framework, then PATH. */
export function sparkleTool(name: string): string | null {
  const candidates = [process.env.SPARKLE_BIN && join(process.env.SPARKLE_BIN, name), join(SPARKLE_TOOLS, name)]
  return candidates.find((path): path is string => Boolean(path) && existsSync(path as string)) ?? Bun.which(name)
}

/** The deltas in `dir`, finished or underway: Sparkle writes each one to `.<name>.delta.tmp` first. */
function deltaNames(dir: string): string[] {
  return readdirSync(dir).flatMap((file) => file.match(/^\.?(.+\.delta)(?:\.tmp)?$/)?.[1] ?? [])
}

export async function generateAppcast(updatesDir: string, downloadURLPrefix: string): Promise<boolean> {
  const tool = sparkleTool("generate_appcast")
  if (!tool) {
    console.error(`generate_appcast not found: run \`swift package resolve\` in macos/, or set SPARKLE_BIN.`)
    return false
  }
  // A delta already in the directory is reused, not built.
  const seen = new Set(deltaNames(updatesDir))
  // The notes prefix makes generate_appcast link Lorca-<version>.md beside an archive as its
  // <sparkle:releaseNotesLink>; Sparkle renders the Markdown in the update window.
  const proc = Bun.spawn(
    [tool, "--download-url-prefix", downloadURLPrefix, "--release-notes-url-prefix", downloadURLPrefix, updatesDir],
    { stdout: "inherit", stderr: "inherit" },
  )
  // generate_appcast prints nothing while it builds a delta, a minute or so each for the CLI
  // binary, so each one is announced as its file appears.
  const poll = setInterval(() => {
    for (const name of deltaNames(updatesDir)) {
      if (seen.has(name)) continue
      seen.add(name)
      log(color.dim(`building ${name}`))
    }
  }, 1000)
  const code = await proc.exited
  clearInterval(poll)
  return code === 0
}

if (import.meta.main) {
  const updatesDir = process.argv[2]
  if (!updatesDir) {
    console.error("usage: bun scripts/generate-appcast.ts <updates-dir>")
    process.exit(1)
  }
  process.exit((await generateAppcast(updatesDir, RELEASES_URL)) ? 0 : 1)
}
