import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite config tuned for Tauri 2: fixed dev port, no auto-open browser
// (Tauri opens its own webview), and HMR over the same port so the desktop
// shell can hot-reload without restarting the Rust process.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
    outDir: "dist",
  },
});
