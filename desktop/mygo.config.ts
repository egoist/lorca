import { defineConfig } from "mygo-cli";
import pkg from "../package.json" with { type: "json" };

// Lorca for Windows and Linux. `mygo dev` builds Lorca Dev (app.lorca.dev), which keeps its
// account in ~/.lorca-dev and its CLI on port 4863, apart from an installed Lorca.
export default defineConfig(({ command }) => ({
  name: "Lorca",
  identifier: "app.lorca",
  // One version for every app: the root package.json's, as the macOS build uses.
  version: pkg.version,
  icon: command === "dev" ? "assets/icon-dev.png" : "assets/icon.png",
  devUrl: "http://localhost:5178",
  devCommand: "bun run dev:web",
  buildCommand: "bun run build:web",
  frontendDist: "dist",
  bindings: "src/mygo.ts",
  out: "build",
  linux: {
    comment: "Chat with your bots, which run on computers you own",
    categories: ["Network", "Chat"],
  },
}));
