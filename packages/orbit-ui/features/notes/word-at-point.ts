// Which character of a turn's text a pointer is on. The text stays plain text
// in its paragraph (no element per word): a long meeting keeps a small
// document, the accessibility tree reads each turn whole, and selecting text
// with the mouse or the keyboard works as in any paragraph.

/** The text offset under a screen point, as the browser places its caret there. */
function caretAt(x: number, y: number): { node: Node; offset: number } | null {
  const view = document as Document & {
    caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
    caretRangeFromPoint?: (x: number, y: number) => Range | null;
  };
  const position = view.caretPositionFromPoint?.(x, y);
  if (position) return { node: position.offsetNode, offset: position.offset };
  const range = view.caretRangeFromPoint?.(x, y);
  return range ? { node: range.startContainer, offset: range.startOffset } : null;
}

/** The paragraph's own text nodes, in order: a hint drawn over a word is not its text. */
function textNodes(paragraph: HTMLElement): Text[] {
  const nodes: Text[] = [];
  const walker = document.createTreeWalker(paragraph, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (!node.parentElement?.closest("[aria-hidden='true']")) nodes.push(node as Text);
  }
  return nodes;
}

/** Where `node` at `offset` falls in the paragraph's text, or none outside it. */
export function textOffset(paragraph: HTMLElement, node: Node, offset: number): number | null {
  let total = 0;
  for (const text of textNodes(paragraph)) {
    if (text === node) return total + offset;
    total += text.data.length;
  }
  return null;
}

function covers(node: Text, offset: number, x: number, y: number): boolean {
  if (offset < 0 || offset >= node.data.length) return false;
  const range = document.createRange();
  range.setStart(node, offset);
  range.setEnd(node, offset + 1);
  return Array.from(range.getClientRects()).some(
    (box) => x >= box.left && x <= box.right && y >= box.top && y <= box.bottom,
  );
}

/**
 * The offset in the paragraph's text of the character under the pointer, or
 * none when the pointer is past the end of a line or between lines.
 */
export function offsetAtPoint(paragraph: HTMLElement, x: number, y: number): number | null {
  const caret = caretAt(x, y);
  if (!caret || !(caret.node instanceof Text) || !paragraph.contains(caret.node)) return null;
  // The caret sits between two characters: the pointer is on one of them.
  const on = [caret.offset, caret.offset - 1].find((offset) =>
    covers(caret.node as Text, offset, x, y),
  );
  return on === undefined ? null : textOffset(paragraph, caret.node, on);
}
