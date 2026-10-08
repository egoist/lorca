// Build the Android app for a GitHub release:
//   Rust core → production prebuild in a copy of mobile/ → Gradle release build → signed APK.
//
//   bun run release-android    writes dist/android/lorca-<version>-android.apk
//
// The APK is signed with the release key, a PKCS12 keystore with the alias `lorca`:
// ANDROID_KEYSTORE and ANDROID_KEYSTORE_PASSWORD name it, else the maintainer's copy in
// ~/Library/Application Support/lorca/android-keys (release.keystore, and its password in
// release.keystore.password). Android installs an update only when it is signed with the same
// key as the installed app, so the build refuses any other certificate than CERTIFICATE_SHA256.
// The version is `version` in mobile/app.config.ts, and the versionCode is made from it.
// The Release phone app workflow (.github/workflows/release-mobile.yml) runs this and uploads the
// APK to the release mobile-v<version>; docs/releasing-mobile.md has the steps.
import { $ } from "bun"
import { existsSync, readdirSync } from "node:fs"
import { copyFile, mkdir, rm } from "node:fs/promises"
import { homedir } from "node:os"
import { join } from "node:path"
import { color, log, ROOT } from "./app.ts"

function die(message: string): never {
  log(color.red(message))
  process.exit(1)
}

if (process.argv.length > 2) die(`unknown argument: ${process.argv[2]}`)

const PACKAGE = "app.lorca"
// SHA-256 of the release key's certificate, as apksigner prints it.
const CERTIFICATE_SHA256 = "c2e73615dcd09e5580c536b9511b00e89d816b1d18a2aeed71bbef0daa8bd16c"
// The ABIs the Rust core is built for (modules/lorca-core/build.ts): phones, and x86_64 for
// Chromebooks and the emulator. React Native's own libraries follow, so the APK holds no ABI the
// core cannot load on.
const ABIS = "arm64-v8a,x86_64"

const MOBILE = join(ROOT, "mobile")
const BUILD_DIR = join(ROOT, "dist", "android")
// The production project is generated in a copy, so mobile/android stays the dev loop's Lorca Dev
// project.
const PROJECT = join(BUILD_DIR, "mobile")
const ANDROID = join(PROJECT, "android")

for (const tool of ["cargo", "cargo-ndk", "bunx", "rsync"]) {
  if (!Bun.which(tool)) die(`missing required tool: ${tool}`)
}

const keysDir = join(homedir(), "Library", "Application Support", "lorca", "android-keys")
const keystore = process.env.ANDROID_KEYSTORE ?? join(keysDir, "release.keystore")
if (!existsSync(keystore)) die(`no release keystore at ${keystore}: docs/releasing-mobile.md`)
const passwordFile = `${keystore}.password`
const password =
  process.env.ANDROID_KEYSTORE_PASSWORD ?? (existsSync(passwordFile) ? (await Bun.file(passwordFile).text()).trim() : "")
if (!password) die(`set ANDROID_KEYSTORE_PASSWORD, or put the keystore's password in ${passwordFile}`)

const sdk = process.env.ANDROID_HOME ?? join(homedir(), "Library", "Android", "sdk")
if (!existsSync(sdk)) die(`no Android SDK at ${sdk}: set ANDROID_HOME`)
const buildTools = readdirSync(join(sdk, "build-tools")).sort((a, b) => a.localeCompare(b, undefined, { numeric: true })).at(-1)
if (!buildTools) die(`no build-tools in ${sdk}`)
const apksigner = join(sdk, "build-tools", buildTools, "apksigner")
const aapt2 = join(sdk, "build-tools", buildTools, "aapt2")
// Android Studio's runtime when no JDK is named, as on a Mac that builds from Android Studio.
const studioJava = "/Applications/Android Studio.app/Contents/jbr/Contents/Home"
const javaHome = process.env.JAVA_HOME ?? (existsSync(studioJava) ? studioJava : undefined)
if (!javaHome) die("set JAVA_HOME to a JDK 17 or later")

const env: Record<string, string> = {
  ...(process.env as Record<string, string>),
  ANDROID_HOME: sdk,
  JAVA_HOME: javaHome,
  // Gradle takes project properties from the environment, which keeps the password out of the
  // command line. These are the ones Android Studio's signed builds use; they sign every variant.
  "ORG_GRADLE_PROJECT_android.injected.signing.store.file": keystore,
  "ORG_GRADLE_PROJECT_android.injected.signing.store.password": password,
  "ORG_GRADLE_PROJECT_android.injected.signing.key.alias": "lorca",
  "ORG_GRADLE_PROJECT_android.injected.signing.key.password": password,
}
// The variant variables would make a Lorca Dev build.
delete env.LORCA_MOBILE_VARIANT
delete env.EAS_BUILD_PROFILE

const config = (await import(join(MOBILE, "app.config.ts"))).default()
const version: string = config.version
const versionCode: number = config.android.versionCode
if (!/^\d+\.\d+\.\d+$/.test(version)) die(`the version in mobile/app.config.ts is "${version}", not X.Y.Z`)

// ---- 1. Rust core
// The shared libraries are gitignored and the dev loop may hold older ones, so a release builds its own.
log(`${color.bold("core")} ${color.dim("release build for Android")}`)
await $`bun run core android`.cwd(MOBILE).env(env)

// ---- 2. production project
log(`${color.bold("prebuild")} ${color.dim(`Lorca ${version} (${versionCode}), ${PACKAGE}`)}`)
await mkdir(PROJECT, { recursive: true })
// The native projects and prebuild's cache in .expo are the copy's own.
await $`rsync -a --delete --exclude /ios --exclude /android --exclude /.expo ${`${MOBILE}/`} ${`${PROJECT}/`}`
await rm(ANDROID, { recursive: true, force: true })
await $`bunx expo prebuild --platform android --no-install`.cwd(PROJECT).env(env)

// ---- 3. release build
log(`${color.bold("building")} ${color.dim(`${ABIS}, signed with ${keystore}`)}`)
// Native libraries go in compressed: the APK is downloaded whole for every update, and Android
// unpacks only the phone's ABI when it installs.
await $`./gradlew :app:assembleRelease -PreactNativeArchitectures=${ABIS} -Pexpo.useLegacyPackaging=true`.cwd(ANDROID).env(env)
const built = join(ANDROID, "app", "build", "outputs", "apk", "release", "app-release.apk")
if (!existsSync(built)) die("Gradle produced no APK")

// ---- 4. check what was built
const badging = await $`${aapt2} dump badging ${built}`.env(env).text()
const pkg = badging.match(/^package: name='([^']+)' versionCode='(\d+)' versionName='([^']+)'/m)
if (!pkg) die("aapt2 printed no package line")
if (pkg[1] !== PACKAGE) die(`the APK is ${pkg[1]}, not ${PACKAGE}`)
if (Number(pkg[2]) !== versionCode || pkg[3] !== version) die(`the APK is ${pkg[3]} (${pkg[2]}), expected ${version} (${versionCode})`)
// One "… certificate SHA-256 digest:" line per signer and signature scheme.
const certs = await $`${apksigner} verify --print-certs ${built}`.env(env).text()
const signers = [...new Set([...certs.matchAll(/certificate SHA-256 digest: ([0-9a-f]+)$/gm)].map((m) => m[1]))]
if (signers.length !== 1 || signers[0] !== CERTIFICATE_SHA256)
  die(`the APK is signed by ${signers.join(", ") || "no certificate"}, not the release key ${CERTIFICATE_SHA256}: installed apps would refuse it`)

const apk = join(BUILD_DIR, `lorca-${version}-android.apk`)
await copyFile(built, apk)
const bytes = await Bun.file(apk).bytes()
const sha256 = new Bun.CryptoHasher("sha256").update(bytes).digest("hex")
log(`${color.green("built")} Lorca ${version} (${versionCode})`)
console.log(`  ${apk}`)
console.log(`  ${(bytes.length / 1024 / 1024).toFixed(1)} MB, sha256 ${sha256}`)
