/// <reference types="vitest/config" />
import { mergeConfig, defineConfig } from "vitest/config";
import viteConfig from "./vite.config";

// Reuse the app's Vite config (React + JSX transform) and layer the test
// environment on top. Component tests run in jsdom; Tauri + automerge are
// mocked per-test, so no native WASM is loaded.
export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: "jsdom",
      globals: true,
      setupFiles: ["./src/test/setup.ts"],
    },
  })
);
