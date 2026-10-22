import { type RefObject, useCallback, useState } from "react";

/** The selected words of one turn, for «Corregir «…»». */
export function useWordSelection(container: RefObject<HTMLDivElement | null>) {
  const [selected, setSelected] = useState<{ text: string; turn: number } | null>(null);
  const capture = () => {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount !== 1 || selection.isCollapsed) {
      setSelected(null);
      return;
    }
    const range = selection.getRangeAt(0);
    const start =
      range.startContainer.nodeType === Node.ELEMENT_NODE
        ? (range.startContainer as Element)
        : range.startContainer.parentElement;
    const paragraph = start?.closest("article p");
    const turn = paragraph?.closest("article")?.getAttribute("data-turn");
    const inside =
      paragraph != null &&
      container.current?.contains(paragraph) === true &&
      paragraph.contains(range.endContainer);
    const text = selection.toString().trim();
    setSelected(inside && text !== "" && turn != null ? { text, turn: Number(turn) } : null);
  };
  const clear = useCallback(() => setSelected(null), []);
  return { selected, capture, clear };
}
