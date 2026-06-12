import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import wasm from "vite-plugin-wasm";

export default defineConfig({
  plugins: [react(), wasm()],
  optimizeDeps: {
    // automerge uses WASM; exclude from Vite's pre-bundling
    exclude: ["@automerge/automerge-wasm", "@automerge/automerge"],
  },
  build: {
    target: "esnext",
  },
});
