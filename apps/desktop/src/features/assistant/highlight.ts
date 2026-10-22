import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

/**
 * The languages the chat colours, by the names models write after the fence.
 * Each grammar is its own chunk, loaded the first time a block asks for it;
 * a language not listed is shown as plain text. The JavaScript regex engine
 * avoids loading Oniguruma's WebAssembly.
 */
const GRAMMARS = {
  typescript: () => import("shiki/langs/typescript.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  shellscript: () => import("shiki/langs/shellscript.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  swift: () => import("shiki/langs/swift.mjs"),
  html: () => import("shiki/langs/html.mjs"),
  css: () => import("shiki/langs/css.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
  diff: () => import("shiki/langs/diff.mjs"),
} as const;

type Grammar = keyof typeof GRAMMARS;

const ALIASES: Record<string, Grammar> = {
  ts: "typescript",
  js: "javascript",
  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  shell: "shellscript",
  console: "shellscript",
  py: "python",
  rs: "rust",
  yml: "yaml",
  md: "markdown",
};

/** One coloured piece of a line: its text and its colour in each of Orbit's themes. */
export interface CodeToken {
  content: string;
  style: Record<string, string>;
}

let highlighter: Promise<HighlighterCore> | null = null;

function grammarOf(language: string): Grammar | null {
  const name = language.toLowerCase();
  if (Object.hasOwn(GRAMMARS, name)) return name as Grammar;
  return ALIASES[name] ?? null;
}

/**
 * The lines of `code` in coloured pieces, or null when its language is not
 * one the chat colours. Each piece carries both themes' colours as CSS
 * variables (`--shiki-dark`, `--shiki-light`); the stylesheet picks one.
 */
export async function highlight(code: string, language: string): Promise<CodeToken[][] | null> {
  const grammar = grammarOf(language);
  if (grammar === null) return null;
  highlighter ??= createHighlighterCore({
    themes: [import("shiki/themes/github-light.mjs"), import("shiki/themes/github-dark.mjs")],
    langs: [],
    engine: createJavaScriptRegexEngine(),
  });
  const core = await highlighter;
  if (!core.getLoadedLanguages().includes(grammar)) {
    await core.loadLanguage(GRAMMARS[grammar]());
  }
  const { tokens } = core.codeToTokens(code, {
    lang: grammar,
    themes: { light: "github-light", dark: "github-dark" },
    defaultColor: false,
  });
  return tokens.map((line) =>
    line.map((token) => ({ content: token.content, style: token.htmlStyle ?? {} })),
  );
}
