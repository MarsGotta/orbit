import { describe, expect, it } from "vitest";
import { type Block, parseInline, parseMarkdown, safeHref } from "./markdown";

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
