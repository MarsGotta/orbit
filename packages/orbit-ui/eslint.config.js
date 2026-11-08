import eslint from "@eslint/js";
import tseslint from "typescript-eslint";

const sourceRules = {
  "@typescript-eslint/no-explicit-any": "error",
  "max-lines": ["error", { max: 200, skipBlankLines: true, skipComments: true }],
  "max-lines-per-function": ["error", { max: 60, skipBlankLines: true, skipComments: true }],
};

// The package is what any frame can show, so nothing here may name a frame.
// Inside the package imports are relative; only a frame uses its name.
// noInlineConfig: a comment cannot switch this off.
const boundaries = {
  patterns: [
    {
      group: ["@tauri-apps/*"],
      message: "The shared package may not import Tauri: that belongs to a frame.",
    },
    {
      group: ["orbit-ui", "orbit-ui/*"],
      message: "Inside the package imports are relative, not by the package's own name.",
    },
    {
      regex: String.raw`(?:^|/)(?:apps/desktop|src-tauri)(?:/|$)|^(?:\.\./)+(?:app|features|showcase)(?:/|$)`,
      message: "The shared package may not import a frame (apps/desktop, its views or its shell).",
    },
    {
      regex: String.raw`^\./\.\.|(?:^|/)(?!\.\.(?:/|$))[^/]+/\.\.(?:/|$)`,
      message: 'Write the import\'s path without "./.." or "<folder>/..".',
    },
  ],
};

export default tseslint.config(
  { ignores: ["node_modules", "bridge/generated"] },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      globals: {
        document: "readonly",
        navigator: "readonly",
        window: "readonly",
        structuredClone: "readonly",
        setTimeout: "readonly",
      },
    },
    linterOptions: { noInlineConfig: true },
    rules: { ...sourceRules, "no-restricted-imports": ["error", boundaries] },
  },
  {
    files: ["**/*.test.{ts,tsx}"],
    rules: {
      "max-lines": "off",
      "max-lines-per-function": "off",
    },
  },
  {
    files: ["state/**/*.{ts,tsx}"],
    languageOptions: { parserOptions: { projectService: true } },
    rules: {
      "@typescript-eslint/switch-exhaustiveness-check": "error",
      "@typescript-eslint/strict-boolean-expressions": "error",
    },
  },
);
