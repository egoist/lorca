import { APP_NAME, buildApp, bundlePath, color, log } from "./app.ts"

const config = process.argv.includes("--debug") ? "debug" : "release"

log(`${color.bold("building")} ${color.dim(`${APP_NAME} (${config})`)}`)
const result = await buildApp(config)

if (!result.ok) {
  log(color.red("build failed"))
  process.exit(1)
}

log(`${color.green("built")} ${bundlePath(config)} ${color.dim(`in ${Math.round(result.ms)}ms`)}`)
