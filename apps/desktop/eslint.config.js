import eslint from "@eslint/js";
import tseslint from "typescript-eslint";

const sourceRules = {
  "@typescript-eslint/no-explicit-any": "error",
  "max-lines": ["error", { max: 200, skipBlankLines: true, skipComments: true }],
  "max-lines-per-function": ["error", { max: 60, skipBlankLines: true, skipComments: true }],
};

export default tseslint.config(
  { ignores: ["dist", "node_modules", "src/bridge/generated", "src-tauri/gen"] },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      globals: {
        document: "readonly",
        navigator: "readonly",
        window: "readonly",
        structuredClone: "readonly",
        setTimeout: "readonly",
      },
    },
    rules: sourceRules,
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    ignores: ["src/bridge/**/*"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/*"],
              message: "Tauri solo puede importarse desde src/bridge/.",
            },
          ],
        },
      ],
    },
  },
  {
    // The showcase (`showcase.html`) is the app with an invented daemon: it
    // installs that daemon behind Tauri's IPC, so it imports Tauri's mocks.
    files: ["src/showcase/**/*.{ts,tsx}"],
    rules: { "no-restricted-imports": "off" },
  },
  {
    // Node scripts; the capture also runs functions inside the page.
    files: ["scripts/**/*.mjs"],
    languageOptions: {
      globals: {
        URL: "readonly",
        console: "readonly",
        document: "readonly",
        getComputedStyle: "readonly",
        process: "readonly",
        requestAnimationFrame: "readonly",
        window: "readonly",
      },
    },
  },
  {
    files: ["src/**/*.test.{ts,tsx}"],
    rules: {
      "max-lines": "off",
      "max-lines-per-function": "off",
    },
  },
  {
    files: ["src/features/**/*.{ts,tsx}", "src/state/**/*.{ts,tsx}"],
    languageOptions: { parserOptions: { projectService: true } },
    rules: {
      "@typescript-eslint/switch-exhaustiveness-check": "error",
      "@typescript-eslint/strict-boolean-expressions": "error",
    },
  },
);
