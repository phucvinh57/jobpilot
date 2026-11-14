import { fileURLToPath } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

// The Tauri shell loads `dist/` (release) or this dev server on port 1420 (`bun run tauri dev`).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": here("./src"), "@assets": here("./assets") } },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    // The bundled fonts live in the repository's `assets/fonts`.
    fs: { allow: [here("./src"), here("./assets"), here("./index.html")] },
    // The Rust side rebuilds through the Tauri CLI, not through Vite.
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**", "**/.claude/**"] },
  },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
  test: { environment: "jsdom", include: ["src/**/*.test.{ts,tsx}"], restoreMocks: true },
});
