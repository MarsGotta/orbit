import { readdirSync } from "node:fs";
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
      regex: String.raw`(?:^|/)(?:apps/desktop|src-tauri)(?:/|$)|^(?:\.\./)+(?:app|showcase)(?:/|$)`,
      message: "The shared package may not import a frame (apps/desktop, its views or its shell).",
    },
    {
      regex: String.raw`^\./\.\.|(?:^|/)(?!\.\.(?:/|$))[^/]+/\.\.(?:/|$)`,
      message: 'Write the import\'s path without "./.." or "<folder>/..".',
    },
  ],
};

// Zones. A view is a folder of features/, read here so that a new view joins
// the zones without touching this file. The panel and the welcome stay in the
// Mac's frame: a shared view may not name them. Each block repeats the shared
// patterns: for a file, the last block naming a rule decides it.
const views = readdirSync(`${import.meta.dirname}/features`, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name);
const frameViews = ["call-panel", "onboarding"];
const UP = String.raw`^(?:\.\./)+(?:features/)?`;

// Modules inside a view that other views import today, as if they were its
// entry. A view has no entry yet; this list may only shrink.
export const PHOTO = [
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

function viewZone(view) {
  const others = views.filter((other) => other !== view);
  const patterns = [
    ...boundaries.patterns,
    {
      regex: `(?!${UP}(?:${PHOTO.join("|")})$)${UP}(?:${others.join("|")})/(?!index$).+`,
      message: "A view may use another view only through its entry (features/<view>).",
    },
    {
      regex: `${UP}(?:${frameViews.join("|")})(?:/|$)`,
      message: "A shared view may not import the floating panel or the welcome.",
    },
  ];
  return {
    files: [`features/${view}/**/*.{ts,tsx}`],
    ignores: testFiles,
    linterOptions: { noInlineConfig: true },
    rules: { "no-restricted-imports": ["error", { patterns }] },
  };
}

// What the views stand on may not depend on them. The copy's tests are the
// exception: they mount real screens to read their words.
const belowViewsZone = {
  files: ["state", "design", "bridge", "shell", "i18n"].map((folder) => `${folder}/**/*.{ts,tsx}`),
  ignores: ["i18n/**/*.test.{ts,tsx}"],
  linterOptions: { noInlineConfig: true },
  rules: {
    "no-restricted-imports": [
      "error",
      {
        patterns: [
          ...boundaries.patterns,
          {
            regex: String.raw`^(?:\.\./)+features(?:/|$)`,
            message: "The state, design, bridge, shell and copy may not import a view.",
          },
        ],
      },
    ],
  },
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
  ...views.map(viewZone),
  belowViewsZone,
  {
    files: ["features/**/*.{ts,tsx}", "state/**/*.{ts,tsx}"],
    languageOptions: { parserOptions: { projectService: true } },
    rules: {
      "@typescript-eslint/switch-exhaustiveness-check": "error",
      "@typescript-eslint/strict-boolean-expressions": "error",
    },
  },
);
