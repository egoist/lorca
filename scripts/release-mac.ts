// Release the Mac app:
//   build → Developer ID sign → .dmg → notarize → staple → Sparkle archive → appcast → R2.
//
//   bun run release-mac 0.2.0        bump "version" in package.json, then release
//   bun run release-mac              release the version package.json holds
//   bun run release-mac --local      build, sign, and notarize; publish nothing
//   FORCE=1 bun run release-mac      replace a version that is already published
//   NO_HISTORY=1 bun run release-mac skip the old archives (a full download, no deltas)
//
// docs/releasing-mac.md has the one-time setup.
import { $ } from "bun"
import { existsSync } from "node:fs"
import { mkdir, rm } from "node:fs/promises"
import { join } from "node:path"
import {
  APP_NAME,
  buildApp,
  bundlePath,
  color,
  FEED_URL,
  log,
  PACKAGE_JSON,
  readVersion,
  RELEASES_URL,
  ROOT,
  VERSION_FIELD,
} from "./app.ts"
import { extractReleaseNotes } from "./changelog.ts"
import { generateAppcast } from "./generate-appcast.ts"

process.chdir(ROOT)

function die(message: string): never {
  log(color.red(message))
  process.exit(1)
}

const args = process.argv.slice(2)
const local = args.includes("--local")
const unknown = args.find((arg) => arg.startsWith("--") && arg !== "--local")
if (unknown) die(`unknown option: ${unknown}`)
const positional = args.filter((arg) => !arg.startsWith("--"))
if (positional.length > 1) die("expected at most one version")

const NOTARY_PROFILE = process.env.NOTARY_PROFILE ?? "NOTARY"
// A partial name matches while the keychain holds one Developer ID Application certificate.
const SIGN_IDENTITY = process.env.SIGN_IDENTITY ?? "Developer ID Application"
const R2_DEST = `${process.env.R2_REMOTE ?? "r2"}:${process.env.R2_BUCKET ?? "lorca-mac-releases"}`
// A bucket-scoped R2 token cannot create buckets, which rclone otherwise checks before an upload.
const RCLONE_FLAGS = ["--s3-no-check-bucket"]
// How many earlier archives generate_appcast builds deltas against.
const HISTORY_COUNT = Number(process.env.HISTORY_COUNT ?? "15")
if (!Number.isSafeInteger(HISTORY_COUNT) || HISTORY_COUNT < 0) die("HISTORY_COUNT must be a non-negative integer")

for (const tool of ["cargo", "swift", "ditto", "plutil", "xcrun", "create-dmg", ...(local ? [] : ["rclone"])]) {
  if (!Bun.which(tool)) die(`missing required tool: ${tool}`)
}

const isVersion = (value: string) => /^\d+\.\d+\.\d+$/.test(value)
const newestFirst = (a: string, b: string) => new Intl.Collator("en", { numeric: true }).compare(b, a)

// ---- 1. version
if (positional[0]) {
  if (!isVersion(positional[0])) die(`not a version: ${positional[0]} (expected x.y.z)`)
  // Rewriting the one line keeps the file's formatting, so the release commit is a one-line diff.
  const manifest = await Bun.file(PACKAGE_JSON).text()
  await Bun.write(PACKAGE_JSON, manifest.replace(VERSION_FIELD, `$1"${positional[0]}"`))
  log(`bumped package.json to ${positional[0]}`)
}
const version = readVersion()
if (!isVersion(version)) die(`package.json holds version "${version}", which is not x.y.z`)

const BUILD_DIR = join(ROOT, "dist", "mac")
const UPDATES_DIR = join(BUILD_DIR, "updates")
const zipName = `${APP_NAME}-${version}.zip`
const dmgName = `${APP_NAME}-${version}.dmg`
const dmgPath = join(BUILD_DIR, dmgName)

if (!local && process.env.FORCE !== "1") {
  const published = await $`rclone lsf ${R2_DEST} ${RCLONE_FLAGS}`.nothrow().quiet().text()
  if (published.split("\n").includes(zipName)) die(`${zipName} is already published: pass a new version, or set FORCE=1`)
}

// ---- 2. build and sign
log(`${color.bold("building")} ${color.dim(`${APP_NAME} ${version}`)}`)
const app = bundlePath("release")
// A clean bundle: nothing from an earlier build rides along into the archive. buildApp compiles
// the Rust CLI (`cargo build --release -p lorca`) and the Markdown library, then the Swift app,
// copies the CLI into Contents/Resources/bin, embeds Sparkle, and signs.
await rm(app, { recursive: true, force: true })
const build = await buildApp("release", { signIdentity: SIGN_IDENTITY, onStep: (step) => log(color.dim(step)) })
if (!build.ok) die("build failed")

const plist = join(app, "Contents", "Info.plist")
const built = (await $`plutil -extract CFBundleShortVersionString raw ${plist}`.text()).trim()
if (built !== version) die(`built ${built}, expected ${version}`)

// ---- 3. disk image
log(`packing ${dmgName}`)
const staging = join(BUILD_DIR, "dmg")
await rm(staging, { recursive: true, force: true })
await rm(dmgPath, { force: true })
await mkdir(staging, { recursive: true })
await $`ditto ${app} ${join(staging, `${APP_NAME}.app`)}`
// create-dmg exits non-zero over Finder-scripting hiccups with the image intact, so the file decides.
await $`create-dmg --volname ${`${APP_NAME} ${version}`} --window-size 540 380 --icon-size 128 --icon ${`${APP_NAME}.app`} 150 195 --app-drop-link 390 195 --hide-extension ${`${APP_NAME}.app`} --no-internet-enable ${dmgPath} ${staging}`
  .nothrow()
  .quiet()
if (!existsSync(dmgPath)) die("create-dmg produced no disk image")
await $`codesign --force --timestamp --sign ${SIGN_IDENTITY} ${dmgPath}`

// ---- 4. notarize and staple
// Notarizing the disk image notarizes the code inside it, so one submission staples both.
log(`notarizing ${color.dim(`profile ${NOTARY_PROFILE}`)}`)
await $`xcrun notarytool submit ${dmgPath} --keychain-profile ${NOTARY_PROFILE} --wait`
await $`xcrun stapler staple ${dmgPath}`
await $`xcrun stapler staple ${app}`
await $`codesign --verify --deep --strict ${app}`
await $`spctl --assess --type execute ${app}`

if (local) {
  log(`${color.green("built")} ${APP_NAME} ${version}; nothing was published`)
  console.log(`  app      ${app}\n  download ${dmgPath}`)
  process.exit(0)
}

// ---- 5. Sparkle archive, beside the recent ones
await rm(UPDATES_DIR, { recursive: true, force: true })
await mkdir(UPDATES_DIR, { recursive: true })

if (process.env.NO_HISTORY !== "1") {
  const listing = await $`rclone lsjson ${R2_DEST} ${RCLONE_FLAGS} --files-only --include ${"*.zip"} --include ${"appcast.xml"}`.text()
  const names = (JSON.parse(listing) as { Name: string }[]).map((file) => file.Name)
  const archiveVersion = (name: string) => name.slice(`${APP_NAME}-`.length, -".zip".length)
  const recent = names
    .filter((name) => name.startsWith(`${APP_NAME}-`) && name.endsWith(".zip") && name !== zipName)
    .sort((a, b) => newestFirst(archiveVersion(a), archiveVersion(b)))
    .slice(0, HISTORY_COUNT)
  // The published appcast comes along, so versions older than the pulled archives keep their entries.
  const history = [...(names.includes("appcast.xml") ? ["appcast.xml"] : []), ...recent]
  if (history.length > 0) {
    await $`rclone copy ${R2_DEST} ${UPDATES_DIR} ${RCLONE_FLAGS} ${history.flatMap((name) => ["--include", `/${name}`])}`
  }
  log(recent.length > 0 ? `deltas against ${recent.join(", ")}` : "no earlier archives: this update is a full download")
}

await $`ditto -c -k --keepParent ${app} ${join(UPDATES_DIR, zipName)}`

const changelog = Bun.file(join(ROOT, "CHANGELOG.md"))
const notes = (await changelog.exists()) ? extractReleaseNotes(await changelog.text(), version) : null
if (notes) await Bun.write(join(UPDATES_DIR, `${APP_NAME}-${version}.md`), `${notes}\n`)
else log(color.yellow(`CHANGELOG.md has no "${version}" section: releasing without notes`))

// ---- 6. appcast
if (!(await generateAppcast(UPDATES_DIR, RELEASES_URL))) die("generate_appcast failed")

// ---- 7. upload
// An archive never changes under its name. The appcast changes with every release.
const IMMUTABLE = "Cache-Control: public, max-age=31536000, immutable"
const FRESH = "Cache-Control: public, max-age=300, must-revalidate"
log(`uploading to ${R2_DEST}`)
await $`rclone copyto ${dmgPath} ${`${R2_DEST}/${dmgName}`} ${RCLONE_FLAGS} --header-upload ${IMMUTABLE} --progress`
await $`rclone copy ${UPDATES_DIR} ${R2_DEST} ${RCLONE_FLAGS} --exclude ${"appcast.xml"} --exclude ${"old_updates/**"} --header-upload ${IMMUTABLE} --progress`
await $`rclone copyto ${join(UPDATES_DIR, "appcast.xml")} ${`${R2_DEST}/appcast.xml`} ${RCLONE_FLAGS} --header-upload ${FRESH}`

const dirty = (await $`git status --porcelain package.json CHANGELOG.md`.nothrow().text()).trim()
if (dirty) log(color.yellow("package.json or CHANGELOG.md has uncommitted changes: commit them so the tree matches what shipped"))

log(`${color.green("released")} ${APP_NAME} ${version}`)
console.log(`  download ${RELEASES_URL}${dmgName}\n  update   ${RELEASES_URL}${zipName}\n  feed     ${FEED_URL}`)
