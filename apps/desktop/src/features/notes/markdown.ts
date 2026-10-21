/**
 * The markdown Orbit's notes speak (P9-04): the subset the hub's prompt asks
 * the model for and the editor writes back. Paragraphs, headings (three
 * levels), bullet, numbered and task lists, bold, italic, strike, links to
 * http(s) and line breaks inside a paragraph or an item (a run's `\n`).
 * Anything else stays as literal text (`>`, fences, a fourth heading level):
 * no line is dropped and nothing becomes HTML. Pure and without dependencies; `Markdown.tsx` renders it as React
 * nodes and `markdown-doc.ts` maps it to the editor.
 */

export interface Run {
  text: string;
  bold?: boolean;
  italic?: boolean;
  strike?: boolean;
  /** Only ever an http(s) address; see {@link safeHref}. */
  href?: string;
}

export type Block =
  | { kind: "paragraph"; runs: Run[] }
  | { kind: "heading"; level: 1 | 2 | 3; runs: Run[] }
  | { kind: "bullet"; items: Run[][] }
  | { kind: "ordered"; start: number; items: Run[][] }
  | { kind: "tasks"; items: Array<{ checked: boolean; runs: Run[] }> };

/** An address a note may link to: http or https, nothing else. */
export function safeHref(href: string): string | null {
  const trimmed = href.trim();
  if (!/^https?:\/\/[^\s]+$/i.test(trimmed)) return null;
  try {
    const url = new URL(trimmed);
    return url.protocol === "http:" || url.protocol === "https:" ? trimmed : null;
  } catch {
    return null;
  }
}

type Marks = Omit<Run, "text">;

const PUNCTUATION = /[\\*_~[\]()#+\-.!`>]/;
/** A letter or digit: `_` next to one is part of a word, not a mark. */
export const WORD = /[\p{L}\p{N}]/u;

function closing(source: string, from: number, delimiter: string): number {
  let index = from;
  while (index < source.length) {
    if (source[index] === "\\") {
      index += 2;
      continue;
    }
    if (source.startsWith(delimiter, index)) {
      if (delimiter !== "_" || !WORD.test(source[index + 1] ?? "")) return index;
    }
    index += 1;
  }
  return -1;
}

function linkEnd(source: string, from: number): { label: number; end: number } | null {
  const label = closing(source, from + 1, "]");
  if (label < 0 || source[label + 1] !== "(") return null;
  const end = source.indexOf(")", label + 2);
  return end < 0 ? null : { label, end };
}

function push(runs: Run[], text: string, marks: Marks) {
  if (text === "") return;
  const last = runs[runs.length - 1];
  if (
    last &&
    last.bold === marks.bold &&
    last.italic === marks.italic &&
    last.strike === marks.strike &&
    last.href === marks.href
  ) {
    last.text += text;
  } else {
    runs.push({ text, ...marks });
  }
}

/** Inline markdown as runs of text with their marks. */
export function parseInline(source: string, marks: Marks = {}, runs: Run[] = []): Run[] {
  let text = "";
  let index = 0;
  const flush = () => {
    push(runs, text, marks);
    text = "";
  };
  const wrap = (open: number, close: number, add: Marks, width: number) => {
    flush();
    parseInline(source.slice(open + width, close), { ...marks, ...add }, runs);
    index = close + width;
  };
  while (index < source.length) {
    const char = source[index] ?? "";
    const next = source[index + 1] ?? "";
    if (char === "\\" && PUNCTUATION.test(next)) {
      text += next;
      index += 2;
      continue;
    }
    const delimiter = source.startsWith("**", index)
      ? "**"
      : source.startsWith("~~", index)
        ? "~~"
        : char === "*" || (char === "_" && !WORD.test(source[index - 1] ?? ""))
          ? char
          : null;
    if (delimiter && !/\s/.test(source[index + delimiter.length] ?? " ")) {
      const end = closing(source, index + delimiter.length, delimiter);
      if (end > index + delimiter.length) {
        const add: Marks =
          delimiter === "**"
            ? { bold: true }
            : delimiter === "~~"
              ? { strike: true }
              : { italic: true };
        wrap(index, end, add, delimiter.length);
        continue;
      }
    }
    if (char === "[") {
      const link = linkEnd(source, index);
      if (link) {
        const href = safeHref(source.slice(link.label + 2, link.end));
        flush();
        parseInline(
          source.slice(index + 1, link.label),
          href === null ? marks : { ...marks, href },
          runs,
        );
        index = link.end + 1;
        continue;
      }
    }
    text += char;
    index += 1;
  }
  flush();
  return runs;
}

const HEADING = /^(#{1,3})\s+(.*)$/;
const TASK = /^\s*[-*+]\s+\[([ xX])\]\s?(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const ORDERED = /^\s*(\d{1,9})[.)]\s+(.*)$/;

function lineKind(line: string): Block["kind"] | "blank" {
  if (line.trim() === "") return "blank";
  if (HEADING.test(line)) return "heading";
  if (TASK.test(line)) return "tasks";
  if (BULLET.test(line)) return "bullet";
  if (ORDERED.test(line)) return "ordered";
  return "paragraph";
}

/** A line that continues a paragraph or an item: after a line break. */
function appendTo(runs: Run[], line: string) {
  if (runs.length > 0) push(runs, "\n", {});
  parseInline(line.trim(), {}, runs);
}

/** Block markdown, line by line. A line never disappears: what is not a
 * heading or a list item is paragraph text, and a single line break inside a
 * paragraph is kept as one. */
export function parseMarkdown(markdown: string): Block[] {
  const blocks: Block[] = [];
  for (const raw of markdown.replace(/\r\n?/g, "\n").split("\n")) {
    const kind = lineKind(raw);
    const last = blocks[blocks.length - 1];
    if (kind === "blank") {
      if (last && last.kind === "paragraph") blocks.push({ kind: "paragraph", runs: [] });
      continue;
    }
    if (kind === "heading") {
      const match = HEADING.exec(raw);
      const level = (match?.[1]?.length ?? 1) as 1 | 2 | 3;
      blocks.push({ kind: "heading", level, runs: parseInline((match?.[2] ?? "").trim()) });
    } else if (kind === "tasks") {
      const match = TASK.exec(raw);
      const item = { checked: match?.[1] !== " ", runs: parseInline((match?.[2] ?? "").trim()) };
      if (last?.kind === "tasks") last.items.push(item);
      else blocks.push({ kind: "tasks", items: [item] });
    } else if (kind === "bullet") {
      const runs = parseInline((BULLET.exec(raw)?.[1] ?? "").trim());
      if (last?.kind === "bullet") last.items.push(runs);
      else blocks.push({ kind: "bullet", items: [runs] });
    } else if (kind === "ordered") {
      const match = ORDERED.exec(raw);
      const runs = parseInline((match?.[2] ?? "").trim());
      if (last?.kind === "ordered") last.items.push(runs);
      else blocks.push({ kind: "ordered", start: Number(match?.[1] ?? 1), items: [runs] });
    } else if (last?.kind === "paragraph" && last.runs.length > 0) {
      appendTo(last.runs, raw);
    } else if (/^\s+\S/.test(raw) && last && last.kind !== "heading" && last.kind !== "paragraph") {
      // An indented line continues the list item above it.
      const items = last.kind === "tasks" ? last.items.map((item) => item.runs) : last.items;
      const runs = items[items.length - 1];
      if (runs) appendTo(runs, raw);
    } else if (last?.kind === "paragraph") {
      appendTo(last.runs, raw);
    } else {
      blocks.push({ kind: "paragraph", runs: parseInline(raw.trim()) });
    }
  }
  return blocks.filter((block) => block.kind !== "paragraph" || block.runs.length > 0);
}
