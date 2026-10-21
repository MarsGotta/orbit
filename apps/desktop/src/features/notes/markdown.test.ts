import { describe, expect, it } from "vitest";
import { type Block, type Run, parseInline, parseMarkdown, safeHref } from "./markdown";
import { serializeInline, serializeMarkdown } from "./markdown-write";

const once = (markdown: string) => serializeMarkdown(parseMarkdown(markdown));

describe("markdown subset", () => {
  it("parses what the hub writes into blocks and runs", () => {
    const blocks = parseMarkdown(
      "## Resumen\n\nEl **bloqueo** sigue *abierto* y ~~cerrado~~.\n\n- uno\n- dos\n\n1. a\n2. b\n\n- [x] hecho\n- [ ] pendiente\n\nVer [acta](https://example.com/acta).",
    );
    expect(blocks.map((block) => block.kind)).toEqual([
      "heading",
      "paragraph",
      "bullet",
      "ordered",
      "tasks",
      "paragraph",
    ]);
    const paragraph = blocks[1] as Extract<Block, { kind: "paragraph" }>;
    expect(paragraph.runs).toEqual([
      { text: "El " },
      { text: "bloqueo", bold: true },
      { text: " sigue " },
      { text: "abierto", italic: true },
      { text: " y " },
      { text: "cerrado", strike: true },
      { text: "." },
    ]);
    const tasks = blocks[4] as Extract<Block, { kind: "tasks" }>;
    expect(tasks.items.map((item) => item.checked)).toEqual([true, false]);
    const link = blocks[5] as Extract<Block, { kind: "paragraph" }>;
    expect(link.runs[1]).toEqual({ text: "acta", href: "https://example.com/acta" });
  });

  it("is stable markdown → model → markdown for the subset", () => {
    const corpus = [
      "Texto llano.",
      "# Título\n\nUno *dos* **tres** ~~cuatro~~.",
      "- a\n- b **c**\n\n1. x\n2. y",
      "- [ ] tarea\n- [x] hecha con [enlace](https://orbit.example/x)",
      "snake_case y 2*3 = 6 y un \\* escapado",
      "**_ambas_** y *cursiva*antes y **negrita**después",
      "# no es título\\n",
      "Línea uno\nsigue el párrafo\n\nOtro párrafo",
      "- item\n  continuación del item",
      "> cita\n\n```\ncódigo\n```",
    ];
    for (const markdown of corpus) {
      const first = once(markdown);
      expect(once(first), markdown).toBe(first);
      expect(parseMarkdown(first), markdown).toEqual(parseMarkdown(markdown));
    }
  });

  it("keeps every combination of marks through a round trip", () => {
    const marks: Array<Omit<Run, "text">> = [
      {},
      { bold: true },
      { italic: true },
      { strike: true },
      { bold: true, italic: true },
      { href: "https://example.com/a" },
      { bold: true, href: "https://example.com/b" },
    ];
    const words = ["uno", "dos_tres", "a*b", "[x]", "#4", "~fin"];
    for (const first of marks) {
      for (const second of marks) {
        for (const word of words) {
          const runs: Run[] = [
            { text: word, ...first },
            { text: " y ", ...second },
            { text: "fin", ...first },
          ];
          const markdown = serializeInline(runs);
          const again = serializeInline(parseInline(markdown));
          expect(again, `${JSON.stringify(runs)} → ${markdown}`).toBe(markdown);
          expect(
            parseInline(markdown)
              .map((run) => run.text)
              .join(""),
          ).toBe(`${word} y fin`);
        }
      }
    }
  });

  it("never links outside http(s) and never produces HTML", () => {
    for (const href of [
      "javascript:alert(1)",
      "JaVaScRiPt:alert(1)",
      " javascript:alert(1)",
      "data:text/html,<script>alert(1)</script>",
      "vbscript:msgbox(1)",
      "file:///etc/passwd",
      "//evil.example",
    ]) {
      expect(safeHref(href), href).toBeNull();
      const runs = parseInline(`[pulsa](${href})`);
      expect(
        runs.some((run) => run.href !== undefined),
        href,
      ).toBe(false);
      expect(
        runs
          .map((run) => run.text)
          .join("")
          .startsWith("pulsa"),
      ).toBe(true);
    }
    const hostile = parseMarkdown('<script>alert(1)</script>\n\n<img src=x onerror="alert(1)">');
    expect(hostile).toEqual([
      { kind: "paragraph", runs: [{ text: "<script>alert(1)</script>" }] },
      { kind: "paragraph", runs: [{ text: '<img src=x onerror="alert(1)">' }] },
    ]);
  });

  it("drops no line and stays linear on unmatched delimiters", () => {
    const noisy = "**".repeat(4_000) + "~~".repeat(4_000) + "[".repeat(4_000);
    const started = performance.now();
    const runs = parseInline(noisy);
    expect(runs.map((run) => run.text).join("").length).toBeGreaterThan(0);
    // Generous: a quadratic parser takes seconds here; this one milliseconds.
    expect(performance.now() - started).toBeLessThan(2_000);
  });
});
