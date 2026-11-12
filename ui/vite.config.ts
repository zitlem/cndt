// SPDX-License-Identifier: AGPL-3.0-or-later
import { defineConfig } from "vite";

export default defineConfig({
  // Tauri serves the built files itself, from relative paths.
  base: "./",
  clearScreen: false,
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    // Three.js is large; one bundle is fine for a desktop app.
    chunkSizeWarningLimit: 2000,
  },
});
