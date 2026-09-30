import solid from "@solidjs/vite-plugin";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [solid()],
  server: {
    // The app loads this address during `mygo dev` (devUrl in mygo.config.ts).
    port: 5178,
    strictPort: true,
    // The development app and the packaged builds are not the page's.
    watch: { ignored: ["**/.mygo/**", "**/build/**", "**/*.go"] },
  },
  build: {
    // WebView2 on Windows, WebKitGTK 2.40 and later on Linux.
    target: ["chrome110", "safari16"],
    cssTarget: ["chrome110", "safari16"],
  },
});
