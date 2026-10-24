import { defineConfig } from "vite";

export default defineConfig({
  root: "ui",
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
    // Two pages: the main window and the peek window (paths relative to app/, where Vite runs).
    rollupOptions: { input: { main: "ui/index.html", peek: "ui/peek.html" } },
  },
});
