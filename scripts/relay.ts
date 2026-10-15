// The development relay the dev loops run beside the app (`bun run dev`, `bun run mac:dev`).
import { mkdirSync } from "node:fs"
import { join } from "node:path"
import { ROOT, color, log } from "./app.ts"

export const RELAY_PORT = 8787

let relay: Bun.Subprocess | null = null
let stopping = false

function killPid(pid: number, signal: NodeJS.Signals) {
  try {
    process.kill(pid, signal)
  } catch {
    // Already gone.
  }
}

async function relayAnswers(): Promise<boolean> {
  try {
    return (await fetch(`http://127.0.0.1:${RELAY_PORT}/v1/health`)).ok
  } catch {
    return false
  }
}

/** The pid of a lorca-relay listening on the dev port that this loop did not start. */
async function strayRelayPid(): Promise<number | null> {
  const proc = Bun.spawn(["lsof", "-ti", `tcp:${RELAY_PORT}`, "-sTCP:LISTEN"], { stdout: "pipe", stderr: "pipe" })
  const text = await new Response(proc.stdout).text()
  await proc.exited
  for (const line of text.split("\n")) {
    const pid = Number(line.trim())
    if (!Number.isInteger(pid) || pid <= 0) continue
    const ps = Bun.spawn(["ps", "-o", "command=", "-p", String(pid)], { stdout: "pipe", stderr: "pipe" })
    const command = await new Response(ps.stdout).text()
    await ps.exited
    if (command.includes("lorca-relay")) return pid
  }
  return null
}

/** A local relay on every interface, so a phone on this network can pair through it. The dev
 * app (LORCA_DEV=1) defaults its relay URL to this computer's LAN IP on this port, and the phone
 * that pairs is the development build, so APNs pushes go to `app.lorca.dev`. A relay left over
 * from an earlier loop is replaced: it may predate the blob kinds the CLI now syncs, and the CLI
 * fails every cycle against one that rejects them. Its database lives in `temp/`, out of the
 * build output that mbx prunes on its own: a relay that loses it forgets every paired phone,
 * which then has to pair again. */
export async function startRelay() {
  if (relay) return
  if (await relayAnswers()) {
    const stray = await strayRelayPid()
    if (stray === null) {
      log(`${color.green("relay")} ${color.dim(`already listening on ${RELAY_PORT}`)}`)
      return
    }
    log(`${color.yellow("relay")} ${color.dim(`replacing the one already listening (pid ${stray})`)}`)
    killPid(stray, "SIGTERM")
    for (let attempt = 0; attempt < 40 && (await relayAnswers()); attempt++) await Bun.sleep(50)
  }
  // Build, then run the binary itself. Through `cargo run` the stop signal reaches cargo and
  // not the relay, which lives on orphaned and is what the next loop finds "already listening".
  const build = Bun.spawn(["cargo", "build", "-q", "-p", "lorca-relay"], { cwd: ROOT, stdin: "ignore", stdout: "inherit", stderr: "inherit" })
  if ((await build.exited) !== 0) {
    log(color.red("relay build failed — not started"))
    return
  }
  mkdirSync(join(ROOT, "temp"), { recursive: true })
  relay = Bun.spawn(
    [join(ROOT, "target", "debug", "lorca-relay"), "--bind", `0.0.0.0:${RELAY_PORT}`, "--db", join(ROOT, "temp", "lorca-relay.db"), "--apns-topic", "app.lorca.dev"],
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
  log(`${color.green("relay")} ${color.dim(`0.0.0.0:${RELAY_PORT} · pairing codes carry this computer's LAN IP`)}`)
}

/** Stops the relay this loop started; `final` when the loop itself is stopping. */
export async function stopRelay(final = false) {
  if (final) stopping = true
  const current = relay
  relay = null
  if (!current) return
  current.kill()
  await Promise.race([current.exited, Bun.sleep(2000)])
}

/** A relay source changed: rebuild it and run the new one. The app is untouched; the CLI
 * reconnects on its own. */
export async function restartRelay() {
  log(`${color.bold("relay")} ${color.dim("changed — rebuilding")}`)
  await stopRelay()
  if (!stopping) await startRelay()
}
