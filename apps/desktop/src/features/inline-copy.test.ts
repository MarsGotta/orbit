import { expect, it } from "vitest";
import { inlineCopy } from "../../scripts/inline-copy-scan.mjs";

const sources = import.meta.glob<string>(
  ["./**/*.{ts,tsx}", "../{app,design,state}/**/*.{ts,tsx}"],
  {
    query: "?raw",
    import: "default",
    eager: true,
  },
);
it("keeps interface copy in the typed catalogues", () => {
  const candidates = Object.entries(sources).flatMap(([file, code]) => {
    if (/(\.test\.|\/test[-.]|\/__)/.test(file)) return [];
    const relative = file.startsWith("./") ? `features/${file.slice(2)}` : file.slice(3);
    return inlineCopy(relative, code).map((candidate) => ({ file: relative, ...candidate }));
  });
  expect(candidates).toEqual([]);
});
it.each([
  "<button>Continue</button>",
  'function CodeBlock() { if (query === undefined) return <p role="alert">No se pudo leer esta consulta de tareas.</p>; }',
  'const view = <Markdown components={{code: () => { return <p role="alert">No se pudo leer esta consulta de tareas.</p>; }}} />;',
  'const view = <Rows>{rows.map(() => <p role="alert">Could not read this task query.</p>)}</Rows>',

  '<input placeholder="Search notes" />',
  '<input placeholder="https://sync.example.test" />',
  '<button>{busy ? "working" : "ready"}</button>',
  '<View lead="Choose a provider" />',
  '<button aria-label="Open menu" />',
  'const title = "Continue";',
  "function label(n: number) { return `Found ${n} notes`; }",
])("rejects newly written interface copy: %s", (code) => {
  expect(inlineCopy("features/example.tsx", code)).not.toEqual([]);
  expect(inlineCopy("features/vault/VaultMarkdown.tsx", code)).not.toEqual([]);
});
it("ignores comments, types, ids and CSS attributes", () => {
  expect(
    inlineCopy(
      "features/example.tsx",
      `
    // Continue here
    type Name = "Continue";
    const element = <div className="some readable classes" data-state="Ready" />;
    if (key === "Enter") return null;
  `,
    ),
  ).toEqual([]);
});
