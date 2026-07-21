/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";

// Dedicated Vitest config for the integration suite under `tests/integration`.
//
// The default `vitest run` (vite.config.ts) only globs `src/**/*.{test,spec}.ts`,
// so these files never run as part of the fast unit pass. That is deliberate:
// the delivery workflow (CLAUDE.md) auto-runs unit tests but keeps
// integration/E2E execution opt-in. Run this suite explicitly with
//   npm run test:integration
export default defineConfig({
  plugins: [vue()],

  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },

  test: {
    environment: "jsdom",
    globals: true,
    include: ["tests/integration/**/*.{test,spec}.ts"],
  },
});
