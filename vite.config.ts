import { defineConfig } from "vite";

// Tauri expects a fixed dev-server port and doesn't want Vite clearing the terminal.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**", "**/crates/**"] } },
  build: { target: "safari15", outDir: "dist", emptyOutDir: true },
  test: { include: ["src/**/*.test.ts"] },
});
