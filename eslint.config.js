import js from "@eslint/js";
import pluginVue from "eslint-plugin-vue";
import pluginSecurity from "eslint-plugin-security";
import pluginNoUnsanitized from "eslint-plugin-no-unsanitized";
import { defineConfigWithVueTs, vueTsConfigs } from "@vue/eslint-config-typescript";
import skipFormatting from "@vue/eslint-config-prettier/skip-formatting";

// Flat ESLint config for the Vue 3 + TypeScript renderer.
// Rust/Tauri code is linted separately with clippy (see src-tauri/), so it is ignored here.
export default defineConfigWithVueTs(
  {
    name: "app/files-to-lint",
    files: ["**/*.{ts,mts,tsx,vue,js,mjs}"],
  },
  {
    // Generated, vendored, and build output — never linted.
    name: "app/global-ignores",
    ignores: [
      "dist/**",
      "dist-ssr/**",
      "coverage/**",
      "node_modules/**",
      "src-tauri/**",
      "tests/e2e/artifacts/**",
    ],
  },

  js.configs.recommended,
  ...pluginVue.configs["flat/recommended"],
  vueTsConfigs.recommended,

  // Security: general JS anti-patterns + XSS/DOM-sink detection (renderer runs in a WebView).
  pluginSecurity.configs.recommended,
  pluginNoUnsanitized.configs.recommended,

  {
    name: "app/rules",
    rules: {
      // Allow intentional unused args/vars when prefixed with an underscore.
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // Flags every `obj[variable]` access — extremely high false-positive rate, and
      // TypeScript already type-checks these. We keep the plugin's high-signal rules
      // (eval, child_process, non-literal fs/regexp, unsafe-regex) and drop only this one.
      "security/detect-object-injection": "off",
    },
  },

  // Test files: relax security rules that create noise around fixtures/mocks.
  {
    name: "app/test-overrides",
    files: ["**/*.test.ts", "tests/**/*.{ts,mjs}"],
    rules: {
      "security/detect-object-injection": "off",
      "security/detect-non-literal-fs-filename": "off",
      "security/detect-non-literal-regexp": "off",
    },
  },

  // The Playwright E2E runner is a Node script whose page.evaluate() callbacks
  // execute in the browser — so it legitimately uses both Node and DOM globals.
  {
    name: "app/e2e-globals",
    files: ["tests/e2e/**/*.mjs"],
    languageOptions: {
      globals: {
        process: "readonly",
        console: "readonly",
        fetch: "readonly",
        setTimeout: "readonly",
        clearTimeout: "readonly",
        URL: "readonly",
        document: "readonly",
        window: "readonly",
        getComputedStyle: "readonly",
      },
    },
  },

  // Keep ESLint out of Prettier's lane — must come last.
  skipFormatting,
);
