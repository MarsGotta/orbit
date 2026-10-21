import { describe, expect, it } from "vitest";
import { docToMarkdown, markdownToDoc } from "./markdown-doc";
import { parseMarkdown } from "./markdown";
import { serializeMarkdown } from "./markdown-write";

const corpus = [
  "Texto llano con **negrita**, *cursiva*, ~~tachado~~ y [enlace](https://example.com).",
  "# Uno\n\n## Dos\n\n### Tres",
  "- a\n- b\n\n1. x\n2. y\n\n3. sigue",
  "- [ ] pendiente\n- [x] hecha",
  "snake_case, 2*3 y \\[corchetes\\]",
  "**_ambas_** marcas",
];

describe("markdown ↔ editor", () => {
  it("round-trips the subset through the editor document", () => {
    for (const markdown of corpus) {
      const normal = serializeMarkdown(parseMarkdown(markdown));
      expect(docToMarkdown(markdownToDoc(markdown)), markdown).toBe(normal);
      expect(docToMarkdown(markdownToDoc(normal)), markdown).toBe(normal);
    }
  });

  it("an empty note is an empty paragraph and back to nothing", () => {
    expect(markdownToDoc("")).toEqual({ type: "doc", content: [{ type: "paragraph" }] });
    expect(docToMarkdown(markdownToDoc(""))).toBe("");
  });

  it("keeps the text of nodes outside the subset and refuses unsafe links", () => {
    const markdown = docToMarkdown({
      type: "doc",
      content: [
        {
          type: "blockquote",
          content: [{ type: "paragraph", content: [{ type: "text", text: "cita" }] }],
        },
        {
          type: "paragraph",
          content: [
            {
              type: "text",
              text: "malo",
              marks: [{ type: "link", attrs: { href: "javascript:alert(1)" } }],
            },
          ],
        },
      ],
    });
    expect(markdown).toBe("cita\n\nmalo");
  });
});
