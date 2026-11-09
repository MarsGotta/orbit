import { readdirSync } from "node:fs";
import eslint from "@eslint/js";
import tseslint from "typescript-eslint";
import { PHOTO } from "../../packages/orbit-ui/eslint.config.js";

const sourceRules = {
  "@typescript-eslint/no-explicit-any": "error",
  "max-lines": ["error", { max: 200, skipBlankLines: true, skipComments: true }],
  "max-lines-per-function": ["error", { max: 60, skipBlankLines: true, skipComments: true }],
};

const tauri = {
  group: ["@tauri-apps/*"],
  message: "Tauri may only be imported from src/bridge/.",
};

// Zones. Only the panel and the welcome are views of the Mac's frame; the
// shared views live in the package, which owns their list of entries. Each
// block repeats the Tauri pattern: for a file, the last block naming a rule
// decides it. The zones also set noInlineConfig, so no comment can switch them
// off; that ignores comments for every rule in those folders (their tests stay
// outside).
const frameViews = ["call-panel", "onboarding"];
const sharedViews = readdirSync(`${import.meta.dirname}/../../packages/orbit-ui/features`, {
  withFileTypes: true,
})
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name);
const UP = String.raw`^(?:\.\./)+(?:features/)?`;

const testFiles = ["**/*.test.{ts,tsx}", "**/test-*.{ts,tsx}", "**/__fixtures__/**"];

// The patterns read the path as written: "./.." or "<folder>/.." would hide
// where an import really goes.
const unnormalized = {
  regex: String.raw`^\./\.\.|(?:^|/)(?!\.\.(?:/|$))[^/]+/\.\.(?:/|$)`,
  message: 'Write the import\'s path without "./.." or "<folder>/..".',
};

// From the app the package is named, never reached by path.
const packageByPath = {
  regex: String.raw`(?:^|/)packages/orbit-ui(?:/|$)`,
  message: 'Import the shared package by its name ("orbit-ui/..."), not by a path.',
};

function viewZone(view) {
  const others = frameViews.filter((other) => other !== view);
  const patterns = [
    tauri,
    unnormalized,
    packageByPath,
    {
      regex: `(?!${UP}(?:${PHOTO.join("|")})$)${UP}(?:${others.join("|")})/(?!index$).+`,
      message: "A view may use another view only through its entry (features/<view>).",
    },
    {
      regex: `^orbit-ui/features/(?!(?:${PHOTO.join("|")})$)(?:${sharedViews.join("|")})/(?!index$).+`,
      message: "A view of the frame may use a shared view only through its entry.",
    },
  ];
  return {
    files: [`src/features/${view}/**/*.{ts,tsx}`],
    ignores: testFiles,
    linterOptions: { noInlineConfig: true },
    rules: { "no-restricted-imports": ["error", { patterns }] },
  };
}

export default tseslint.config(
  { ignores: ["dist", "node_modules", "src-tauri/gen"] },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  // Importing the package's list above makes typescript-eslint see two config
  // roots; this one is the app's.
  { languageOptions: { parserOptions: { tsconfigRootDir: import.meta.dirname } } },
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
    rules: { "no-restricted-imports": ["error", { patterns: [tauri] }] },
  },
  ...frameViews.map(viewZone),
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
    // The Mac's own pieces of Settings; a comment cannot switch these rules off.
    files: ["src/settings/**/*.{ts,tsx}"],
    linterOptions: { noInlineConfig: true },
  },
  {
    files: ["src/features/**/*.{ts,tsx}", "src/settings/**/*.{ts,tsx}", "src/state/**/*.{ts,tsx}"],
    languageOptions: { parserOptions: { projectService: true } },
    rules: {
      "@typescript-eslint/switch-exhaustiveness-check": "error",
      "@typescript-eslint/strict-boolean-expressions": "error",
    },
  },
);
