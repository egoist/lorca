// Builds the Rust core for the phone and drops it where the Expo module picks it up: an
// xcframework plus Swift bindings under ios/, shared libraries plus Kotlin bindings under
// android/. Run from mobile/: `bun run core` (or `bun run core ios` / `bun run core android`).
import { existsSync, mkdirSync, readdirSync, renameSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

const MODULE = import.meta.dir;
const ROOT = resolve(MODULE, "../../..");
const TARGET = join(ROOT, "target");
const platforms = process.argv.slice(2).filter((a) => a === "ios" || a === "android");
const wantIos = platforms.length === 0 || platforms.includes("ios");
const wantAndroid = platforms.length === 0 || platforms.includes("android");

async function run(cmd: string[], env: Record<string, string> = {}) {
  console.log("$", cmd.join(" "));
  const proc = Bun.spawn(cmd, { cwd: ROOT, env: { ...process.env, ...env }, stdout: "inherit", stderr: "inherit" });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`${cmd[0]} exited with ${code}`);
}

function ndkHome(): string {
  if (process.env.ANDROID_NDK_HOME) return process.env.ANDROID_NDK_HOME;
  const sdk = process.env.ANDROID_HOME ?? join(homedir(), "Library/Android/sdk");
  const versions = readdirSync(join(sdk, "ndk")).sort();
  const latest = versions.at(-1);
  if (!latest) throw new Error("No Android NDK under " + join(sdk, "ndk"));
  return join(sdk, "ndk", latest);
}

// The host dylib carries the UniFFI metadata the bindings are generated from.
await run(["cargo", "build", "-p", "tinybot-mobile"]);
const bindgen = ["cargo", "run", "-q", "-p", "tinybot-mobile", "--features", "bindgen", "--bin", "uniffi-bindgen", "--", "generate", "--library", join(TARGET, "debug/libtinybot_mobile.dylib")];

if (wantIos) {
  await run(["cargo", "build", "-p", "tinybot-mobile", "--release", "--target", "aarch64-apple-ios"]);
  await run(["cargo", "build", "-p", "tinybot-mobile", "--release", "--target", "aarch64-apple-ios-sim"]);
  const generated = join(MODULE, "ios/generated");
  rmSync(generated, { recursive: true, force: true });
  await run([...bindgen, "--language", "swift", "--out-dir", generated]);
  // The header and its module map go where both the xcframework and the pod find them.
  const include = join(generated, "include");
  mkdirSync(include, { recursive: true });
  renameSync(join(generated, "tinybot_mobileFFI.h"), join(include, "tinybot_mobileFFI.h"));
  renameSync(join(generated, "tinybot_mobileFFI.modulemap"), join(include, "module.modulemap"));
  const xcframework = join(MODULE, "ios/TinybotCore.xcframework");
  rmSync(xcframework, { recursive: true, force: true });
  await run([
    "xcodebuild", "-create-xcframework",
    "-library", join(TARGET, "aarch64-apple-ios/release/libtinybot_mobile.a"), "-headers", include,
    "-library", join(TARGET, "aarch64-apple-ios-sim/release/libtinybot_mobile.a"), "-headers", include,
    "-output", xcframework,
  ]);
}

if (wantAndroid) {
  const jniLibs = join(MODULE, "android/src/main/jniLibs");
  rmSync(jniLibs, { recursive: true, force: true });
  await run(["cargo", "ndk", "-t", "arm64-v8a", "-t", "x86_64", "-o", jniLibs, "build", "-p", "tinybot-mobile", "--release"], { ANDROID_NDK_HOME: ndkHome() });
  const java = join(MODULE, "android/src/main/java");
  rmSync(join(java, "uniffi"), { recursive: true, force: true });
  await run([...bindgen, "--language", "kotlin", "--no-format", "--out-dir", java]);
}

console.log("core built:", [wantIos && "ios", wantAndroid && "android"].filter(Boolean).join(", "));
if (!existsSync(join(MODULE, "ios/TinybotCore.xcframework")) && wantIos) throw new Error("no xcframework");
