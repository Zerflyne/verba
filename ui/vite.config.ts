import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// La finestra e' fissa a 1600x980: non c'e' niente da servire in produzione
// oltre ai file statici che Tauri incorpora.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true, target: "es2021" },
});
