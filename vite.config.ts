import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { strictPort: true, port: 1427, host: "127.0.0.1" },
  build: { outDir: "web-dist", target: "es2022" },
});
