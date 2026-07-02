/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig(({ mode }) => ({
  plugins: [react()],
  define: {
    "process.env.MAX_EMOJI_VERSION": "undefined",
    "process.env.NODE_ENV": JSON.stringify(process.env.NODE_ENV ?? mode)
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    ...(mode === "tauri" ? { hmr: false } : {})
  },
  clearScreen: false,
  test: {
    // e2e/ belongs to Playwright (headless-Chromium DOM tier), not Vitest.
    exclude: ["e2e/**", "node_modules/**", "dist/**"]
  }
}));
