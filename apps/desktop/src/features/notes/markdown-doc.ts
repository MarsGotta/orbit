import type { JSONContent } from "@tiptap/react";
import { type Block, type Run, parseMarkdown, safeHref } from "./markdown";
import { serializeMarkdown } from "./markdown-write";

/**
 * The editor's document and the note's markdown, both ways (P9-04). Only
 * the subset of `markdown.ts` exists in the editor; a node the editor might
 * still hold outside it becomes paragraph text, so nothing typed is lost.
 */

function textNode(run: Run): JSONContent {
  const marks: JSONContent["marks"] = [];
  if (run.bold === true) marks.push({ type: "bold" });
  if (run.italic === true) marks.push({ type: "italic" });
  if (run.strike === true) marks.push({ type: "strike" });
  if (run.href !== undefined) marks.push({ type: "link", attrs: { href: run.href } });
  return marks.length > 0
    ? { type: "text", text: run.text, marks }
    : { type: "text", text: run.text };
}

/** A run's text, its line breaks as the editor's hard breaks. */
function inlineNodes(run: Run): JSONContent[] {
  return run.text.split("\n").flatMap((text, index) => {
    const part = text === "" ? [] : [textNode({ ...run, text })];
    return index === 0 ? part : [{ type: "hardBreak" }, ...part];
  });
}

function paragraph(runs: Run[]): JSONContent {
  const content = runs.flatMap(inlineNodes);
  return content.length > 0 ? { type: "paragraph", content } : { type: "paragraph" };
}

function item(type: "listItem" | "taskItem", runs: Run[], checked?: boolean): JSONContent {
  return {
    type,
    ...(type === "taskItem" ? { attrs: { checked: checked === true } } : {}),
    content: [paragraph(runs)],
  };
}

function blockNode(block: Block): JSONContent {
  switch (block.kind) {
    case "paragraph":
      return paragraph(block.runs);
    case "heading":
      return { ...paragraph(block.runs), type: "heading", attrs: { level: block.level } };
    case "bullet":
      return { type: "bulletList", content: block.items.map((runs) => item("listItem", runs)) };
    case "ordered":
      return {
        type: "orderedList",
        attrs: { start: block.start },
        content: block.items.map((runs) => item("listItem", runs)),
      };
    case "tasks":
      return {
        type: "taskList",
        content: block.items.map((task) => item("taskItem", task.runs, task.checked)),
      };
  }
}

export function markdownToDoc(markdown: string): JSONContent {
  const content = parseMarkdown(markdown).map(blockNode);
  return { type: "doc", content: content.length > 0 ? content : [{ type: "paragraph" }] };
}

function runsOf(node: JSONContent, runs: Run[] = []): Run[] {
  if (node.type === "text") {
    const marks = new Set((node.marks ?? []).map((mark) => mark.type));
    const linked = (node.marks ?? []).find((mark) => mark.type === "link");
    const href =
      typeof linked?.attrs?.href === "string"
        ? (safeHref(linked.attrs.href) ?? undefined)
        : undefined;
    const run: Run = { text: node.text ?? "" };
    if (marks.has("bold")) run.bold = true;
    if (marks.has("italic")) run.italic = true;
    if (marks.has("strike")) run.strike = true;
    if (href !== undefined) run.href = href;
    runs.push(run);
    return runs;
  }
  if (node.type === "hardBreak") {
    runs.push({ text: "\n" });
    return runs;
  }
  for (const child of node.content ?? []) {
    if (runs.length > 0 && child.type === "paragraph") runs.push({ text: "\n" });
    runsOf(child, runs);
  }
  return runs;
}

const LISTS = new Set(["bulletList", "orderedList", "taskList"]);

/**
 * The items of a list, flat: an item's own text, then the items of any list
 * pasted inside it as items of their own, never merged into its text.
 */
function listItems(node: JSONContent): Run[][] {
  return (node.content ?? []).flatMap((item) => {
    const own: Run[] = [];
    const nested: Run[][] = [];
    for (const child of item.content ?? []) {
      if (LISTS.has(child.type ?? "")) nested.push(...listItems(child));
      else {
        if (own.length > 0) own.push({ text: "\n" });
        runsOf(child, own);
      }
    }
    return [own, ...nested];
  });
}

function nodeBlock(node: JSONContent): Block {
  switch (node.type ?? "") {
    case "heading": {
      const level = Math.min(3, Math.max(1, Number(node.attrs?.level ?? 1))) as 1 | 2 | 3;
      return { kind: "heading", level, runs: runsOf(node) };
    }
    case "bulletList":
      return { kind: "bullet", items: listItems(node) };
    case "orderedList":
      return {
        kind: "ordered",
        start: Math.max(1, Number(node.attrs?.start ?? 1)),
        items: listItems(node),
      };
    case "taskList":
      return {
        kind: "tasks",
        items: (node.content ?? []).flatMap((task) =>
          listItems({ content: [task] }).map((runs, index) => ({
            checked: index === 0 && task.attrs?.checked === true,
            runs,
          })),
        ),
      };
    default:
      return { kind: "paragraph", runs: runsOf(node) };
  }
}

export function docToMarkdown(doc: JSONContent): string {
  const blocks = (doc.content ?? [])
    .map(nodeBlock)
    .filter((block) => block.kind !== "paragraph" || block.runs.some((run) => run.text !== ""));
  return serializeMarkdown(blocks);
}
