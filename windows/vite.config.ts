import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri v2: devUrl http://localhost:1420, frontendDist ../dist (relative to src-tauri).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/core/**", "**/target/**", "**/shots/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    outDir: "dist",
    target: "chrome110", // WebView2 is Chromium
    sourcemap: false,
    chunkSizeWarningLimit: 2500,
  },
});
