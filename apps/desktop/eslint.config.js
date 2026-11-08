import { readdirSync } from "node:fs";
import eslint from "@eslint/js";
import tseslint from "typescript-eslint";

const sourceRules = {
  "@typescript-eslint/no-explicit-any": "error",
  "max-lines": ["error", { max: 200, skipBlankLines: true, skipComments: true }],
  "max-lines-per-function": ["error", { max: 60, skipBlankLines: true, skipComments: true }],
};

const tauri = {
  group: ["@tauri-apps/*"],
  message: "Tauri may only be imported from src/bridge/.",
};

// Zones. A view is a folder of src/features/, read here so that a new view
// joins the zones without touching this file. The panel and the welcome
// belong to the Mac's frame; every other view is shared. Each block repeats
// the Tauri pattern: for a file, the last block naming a rule decides it.
// The zones also set noInlineConfig, so no comment can switch them off; that
// ignores comments for every rule in those folders (their tests stay outside).
const views = readdirSync(`${import.meta.dirname}/src/features`, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name);
const frameViews = ["call-panel", "onboarding"];
const UP = String.raw`^(?:\.\./)+(?:features/)?`;

// Modules inside a view that other views import today, as if they were its
// entry. A view has no entry yet; this list may only shrink.
const PHOTO = [
  "assistant/PendingCard",
  "assistant/SignInNotice",
  "assistant/agent-calls",
  "assistant/assistant-route",
  "assistant/assistants",
  "assistant/composer-insert",
  "assistant/conversation-markdown",
  "assistant/use-sign-in",
  "calendar/date",
  "editor/MarkdownEditor",
  "editor/menus",
  "notes/DictionaryPicker",
  "notes/MarkdownView",
  "notes/markdown",
  "settings/AIProviderEditor",
  "settings/ControlHistory",
  "settings/TranscriptionModelChoice",
  "settings/agents/AgentCards",
  "settings/agents/use-agent-config",
  "settings/layout",
  "settings/permission-reread",
  "settings/use-ai-settings",
  "settings/use-primary",
  "vault/VaultSettingsView",
  "vault/api",
  "vault/distillation-format",
  "vault/route",
  "vault/use-distillations",
  "vault/use-vault-read",
];

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
  const shared = !frameViews.includes(view);
  const others = views.filter((other) => other !== view && !(shared && frameViews.includes(other)));
  const patterns = [
    tauri,
    unnormalized,
    packageByPath,
    {
      regex: `(?!${UP}(?:${PHOTO.join("|")})$)${UP}(?:${others.join("|")})/(?!index$).+`,
      message: "A view may use another view only through its entry (features/<view>).",
    },
  ];
  if (shared) {
    patterns.push(
      {
        regex: String.raw`^(?:\.\./)+app(?:/(?!shell-slots$)|$)`,
        message: "A shared view may not import the Mac's frame (src/app/).",
      },
      {
        regex: `${UP}(?:${frameViews.join("|")})(?:/|$)`,
        message: "A shared view may not import the floating panel or the welcome.",
      },
    );
  }
  return {
    files: [`src/features/${view}/**/*.{ts,tsx}`],
    ignores: testFiles,
    linterOptions: { noInlineConfig: true },
    rules: { "no-restricted-imports": ["error", { patterns }] },
  };
}

const stateZone = {
  files: ["src/state/**/*.{ts,tsx}"],
  ignores: testFiles,
  linterOptions: { noInlineConfig: true },
  rules: {
    "no-restricted-imports": [
      "error",
      {
        patterns: [
          tauri,
          unnormalized,
          packageByPath,
          {
            regex: String.raw`^(?:\.\./)+(?:features|app)(?:/|$)`,
            message: "src/state/ may not import a view (src/features/) or the frame (src/app/).",
          },
        ],
      },
    ],
  },
};

export default tseslint.config(
  { ignores: ["dist", "node_modules", "src-tauri/gen"] },
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
    rules: { "no-restricted-imports": ["error", { patterns: [tauri] }] },
  },
  ...views.map(viewZone),
  stateZone,
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
