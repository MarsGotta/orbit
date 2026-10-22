const BLOCKS = new Set([
  "P",
  "LI",
  "H3",
  "H4",
  "H5",
  "H6",
  "PRE",
  "BLOCKQUOTE",
  "TR",
  "FIGURE",
  "DIV",
]);

/**
 * What «Escuchar» reads: the answer's text as it is shown, one line per
 * block, without the controls drawn inside it (`data-speech="skip"`).
 */
export function spokenText(root: HTMLElement): string {
  const lines: string[] = [];
  let line = "";
  const walk = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      line += node.textContent ?? "";
      return;
    }
    if (!(node instanceof HTMLElement) || node.dataset.speech === "skip") return;
    const block = BLOCKS.has(node.tagName);
    if (block && line.trim() !== "") {
      lines.push(line.trim());
      line = "";
    }
    node.childNodes.forEach(walk);
    if (block && line.trim() !== "") {
      lines.push(line.trim());
      line = "";
    }
  };
  walk(root);
  if (line.trim() !== "") lines.push(line.trim());
  return lines.join("\n");
}
