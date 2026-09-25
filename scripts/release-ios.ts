// Release the iPhone app to TestFlight:
//   Rust core → production prebuild in a copy of mobile/ → pods → archive → upload to App Store Connect.
//
//   bun run release-ios                    archive and upload; the build number is the local time
//   bun run release-ios --local            archive only; upload nothing
//   BUILD_NUMBER=42 bun run release-ios    upload under a chosen build number
//
// Signing and the upload go through the Apple account signed in to Xcode (team GJE9R5VE87), with
// automatic provisioning. App Store Connect holds the app record "Lorca" for `app.lorca`. The
// marketing version is `version` in mobile/app.config.ts. A build appears under TestFlight after
// Apple finishes processing it, usually within half an hour.
import { $ } from "bun"
import { existsSync } from "node:fs"
import { mkdir, readdir, rename, rm } from "node:fs/promises"
import { join } from "node:path"
import { color, log, ROOT } from "./app.ts"

function die(message: string): never {
  log(color.red(message))
  process.exit(1)
}

const args = process.argv.slice(2)
const local = args.includes("--local")
const unknown = args.find((arg) => arg !== "--local")
if (unknown) die(`unknown argument: ${unknown}`)

for (const tool of ["cargo", "xcodebuild", "pod", "bunx", "plutil", "rsync"]) {
  if (!Bun.which(tool)) die(`missing required tool: ${tool}`)
}

const TEAM_ID = "GJE9R5VE87"
const BUNDLE_ID = "app.lorca"
const MOBILE = join(ROOT, "mobile")
const BUILD_DIR = join(ROOT, "dist", "ios")
// The production project is generated in a copy, so mobile/ios stays the dev loop's Lorca Dev project.
// The copy stays between releases at the same path: Xcode's compilation cache keys hold absolute
// paths, and the pods installed in it are used again.
const PROJECT = join(BUILD_DIR, "mobile")
const IOS = join(PROJECT, "ios")
// Pods and its lockfile wait here while prebuild writes a new ios/.
const KEPT = join(BUILD_DIR, "kept")
const ARCHIVE = join(BUILD_DIR, "Lorca.xcarchive")
const EXPORT = join(BUILD_DIR, "export")

// App Store Connect wants every upload's build number above the last. Local time as YYYYMMDDHHmm
// only grows, and it names when the build was made.
const now = new Date()
const pad = (n: number) => String(n).padStart(2, "0")
const buildNumber =
  process.env.BUILD_NUMBER ??
  `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}${pad(now.getHours())}${pad(now.getMinutes())}`
if (!/^\d+$/.test(buildNumber)) die(`BUILD_NUMBER must be digits, got "${buildNumber}"`)

// CocoaPods dies on a non-UTF-8 locale, and the CommandLineTools SDK breaks the pod install and
// the build with "unknown architecture" from tapi. The variant variables would make a Lorca Dev build.
const env: Record<string, string> = {
  ...(process.env as Record<string, string>),
  LANG: "en_US.UTF-8",
  LC_ALL: "en_US.UTF-8",
  DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
  LORCA_IOS_BUILD_NUMBER: buildNumber,
}
delete env.LORCA_MOBILE_VARIANT
delete env.EAS_BUILD_PROFILE

// ---- 1. Rust core
// The xcframework is gitignored and the dev loop may hold an older one, so a release builds its own.
log(`${color.bold("core")} ${color.dim("release build for iOS")}`)
await $`bun run core ios`.cwd(MOBILE).env(env)

// ---- 2. production project
log(`${color.bold("prebuild")} ${color.dim(`Lorca, ${BUNDLE_ID}, build ${buildNumber}`)}`)
// The native projects and prebuild's cache in .expo are the copy's own.
if (existsSync(join(PROJECT, "package.json"))) {
  // rsync copies only what changed, and leaves alone the xcframework that expo-modules-jsi's build
  // phase makes in its package and keys to this copy's Pods path.
  const jsi = "/node_modules/expo-modules-jsi/apple"
  await $`rsync -a --delete --exclude /ios --exclude /android --exclude /.expo --exclude ${`${jsi}/.*`} --exclude ${`${jsi}/Products`} ${`${MOBILE}/`} ${`${PROJECT}/`}`
} else {
  // -c clones on APFS, so node_modules costs next to nothing to copy.
  await rm(PROJECT, { recursive: true, force: true })
  await mkdir(PROJECT, { recursive: true })
  for (const name of await readdir(MOBILE)) {
    if (!["ios", "android", ".expo"].includes(name)) await $`cp -cR ${join(MOBILE, name)} ${PROJECT}`
  }
}
// A clean prebuild writes ios/ from the Expo config. The pods installed last time go back in, so
// pod install uses them again and installs only what changed.
const KEEP = ["Pods", "Podfile.lock"]
await mkdir(KEPT, { recursive: true })
for (const name of KEEP) {
  if (!existsSync(join(IOS, name))) continue
  await rm(join(KEPT, name), { recursive: true, force: true })
  await rename(join(IOS, name), join(KEPT, name))
}
await rm(IOS, { recursive: true, force: true })
await $`bunx expo prebuild --platform ios --no-install`.cwd(PROJECT).env(env)
for (const name of KEEP) {
  if (existsSync(join(KEPT, name))) await rename(join(KEPT, name), join(IOS, name))
}
await $`pod install`.cwd(IOS).env(env)

const pbxproj = await Bun.file(join(IOS, "Lorca.xcodeproj", "project.pbxproj")).text()
const bundleIds = new Set([...pbxproj.matchAll(/PRODUCT_BUNDLE_IDENTIFIER = "?([^";]+)"?;/g)].map((m) => m[1]))
if (!bundleIds.has(BUNDLE_ID)) die(`the project builds ${[...bundleIds].join(", ")}, not ${BUNDLE_ID}`)

// ---- 3. archive
// CURRENT_PROJECT_VERSION carries the build number to the notify extension, whose Info.plist reads
// it; the app's own Info.plist has it from the Expo config.
//
// An archive starts from an empty build database, so every compile runs again. Xcode's compilation
// cache (DerivedData/CompilationCache.noindex) answers the C, C++, and Objective-C ones from the
// last release: React Native's libraries drop from about seven minutes to seconds. Swift compiles
// in full, because the cache needs explicit Swift modules and React Native's prebuilt core turns
// them off.
log(`${color.bold("archiving")} ${color.dim(ARCHIVE)}`)
await rm(ARCHIVE, { recursive: true, force: true })
await rm(EXPORT, { recursive: true, force: true })
await $`xcodebuild -workspace ${join(IOS, "Lorca.xcworkspace")} -scheme Lorca -configuration Release -destination generic/platform=iOS -archivePath ${ARCHIVE} -allowProvisioningUpdates CURRENT_PROJECT_VERSION=${buildNumber} COMPILATION_CACHE_ENABLE_CACHING=YES archive -quiet`.env(env)
if (!existsSync(ARCHIVE)) die("xcodebuild produced no archive")

const plist = join(ARCHIVE, "Products", "Applications", "Lorca.app", "Info.plist")
const version = (await $`plutil -extract CFBundleShortVersionString raw ${plist}`.text()).trim()
const built = (await $`plutil -extract CFBundleVersion raw ${plist}`.text()).trim()
if (built !== buildNumber) die(`the app carries build ${built}, expected ${buildNumber}`)

if (local) {
  log(`${color.green("archived")} Lorca ${version} (${buildNumber}); nothing was uploaded`)
  console.log(`  archive ${ARCHIVE}`)
  process.exit(0)
}

// ---- 4. upload
const exportOptions = join(BUILD_DIR, "ExportOptions.plist")
await Bun.write(
  exportOptions,
  `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>upload</string>
  <key>teamID</key><string>${TEAM_ID}</string>
  <key>signingStyle</key><string>automatic</string>
  <key>uploadSymbols</key><true/>
  <key>manageAppVersionAndBuildNumber</key><false/>
</dict>
</plist>
`,
)
log(`${color.bold("uploading")} ${color.dim("to App Store Connect")}`)
await $`xcodebuild -exportArchive -archivePath ${ARCHIVE} -exportOptionsPlist ${exportOptions} -exportPath ${EXPORT} -allowProvisioningUpdates`.env(env)

log(`${color.green("uploaded")} Lorca ${version} (${buildNumber})`)
console.log("  TestFlight lists it once App Store Connect finishes processing")
