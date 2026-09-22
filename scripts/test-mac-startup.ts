import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

const root = resolve(import.meta.dir, "..")
const temporary = await mkdtemp(join(tmpdir(), "lorca-launch-tests-"))
try {
  const executable = join(temporary, "test-launch")
  const compile = Bun.spawn([
    "swiftc", "-parse-as-library", "-swift-version", "5",
    "macos/Tests/LaunchConcurrency.swift",
    "macos/Sources/Lorca/App/CLILaunchWorker.swift",
    "macos/Sources/Lorca/App/StartupTrace.swift",
    "-o", executable,
  ], { cwd: root, stdout: "inherit", stderr: "inherit" })
  if (await compile.exited !== 0) throw new Error("Launch test compilation failed")
  const test = Bun.spawn([executable, temporary], { stdout: "inherit", stderr: "inherit" })
  if (await test.exited !== 0) throw new Error("Launch tests failed")
} finally {
  await rm(temporary, { recursive: true, force: true })
}
