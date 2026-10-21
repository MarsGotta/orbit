import { type Block, type Run, WORD } from "./markdown";

/**
 * Writes the subset of `markdown.ts` back as markdown that parses to the same
 * blocks and runs: every character that would read as a mark is escaped.
 */

function escapeText(text: string, lineStart: boolean): string {
  let escaped = "";
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index] ?? "";
    const before = index === 0 ? "" : (text[index - 1] ?? "");
    const after = text[index + 1] ?? "";
    const boundary = !WORD.test(before) || !WORD.test(after);
    if ("\\*~[]`".includes(char) || (char === "_" && boundary)) escaped += `\\${char}`;
    else escaped += char;
  }
  // A run that begins a line must not read as a block: a heading, a list,
  // a numbered item, or `>` and `---`, which stay literal text.
  if (lineStart && /^(#|[-*+]\s|\d+[.)]\s|>|-{3,}\s*$)/.test(escaped)) escaped = `\\${escaped}`;
  return escaped;
}

function serializeRun(run: Run, lineStart: boolean): string {
  let text = escapeText(run.text, lineStart);
  if (text.trim() === "") return text;
  const lead = /^\s*/.exec(text)?.[0] ?? "";
  const tail = /\s*$/.exec(text)?.[0] ?? "";
  text = text.trim();
  // Italic is `*` unless the run is bold too: `**_x_**` parses back, while
  // `***x***` would not.
  if (run.italic === true) text = run.bold === true ? `_${text}_` : `*${text}*`;
  if (run.bold === true) text = `**${text}**`;
  if (run.strike === true) text = `~~${text}~~`;
  if (run.href !== undefined) text = `[${text}](${run.href})`;
  return `${lead}${text}${tail}`;
}

/** The runs split at their line breaks: a break is a run of its own. */
function lines(runs: Run[]): Run[] {
  return runs.flatMap((run) =>
    run.text.split("\n").flatMap((text, index) => {
      const part = text === "" ? [] : [{ ...run, text }];
      return index === 0 ? part : [{ text: "\n" }, ...part];
    }),
  );
}

/** Inline runs as markdown that parses back to the same runs. */
export function serializeInline(runs: Run[]): string {
  let written = "";
  for (const run of lines(runs)) {
    written +=
      run.text === "\n" ? "\n" : serializeRun(run, written === "" || written.endsWith("\n"));
  }
  return written;
}

/** An item's text: its later lines indented, so they stay in the item. */
const itemText = (runs: Run[]) => serializeInline(runs).replaceAll("\n", "\n  ");

export function serializeMarkdown(blocks: Block[]): string {
  return blocks
    .map((block) => {
      switch (block.kind) {
        case "paragraph":
          return serializeInline(block.runs);
        case "heading":
          return `${"#".repeat(block.level)} ${serializeInline(block.runs)}`;
        case "bullet":
          return block.items.map((runs) => `- ${itemText(runs)}`).join("\n");
        case "ordered":
          return block.items
            .map((runs, index) => `${block.start + index}. ${itemText(runs)}`)
            .join("\n");
        case "tasks":
          return block.items
            .map((item) => `- [${item.checked ? "x" : " "}] ${itemText(item.runs)}`)
            .join("\n");
      }
    })
    .join("\n\n");
}
