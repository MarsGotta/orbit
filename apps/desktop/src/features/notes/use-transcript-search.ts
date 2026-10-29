import { useEffect, useMemo, useRef, useState } from "react";
import { type Match, findMatches } from "./transcript-search";

/**
 * The transcript's search, always at hand: the query, its matches and the one
 * in focus. ⌘F puts the cursor in it; ↵ and ⇧↵ move between matches, and only
 * a match the user moved to scrolls into view (nothing moves on opening).
 */
export function useTranscriptSearch(texts: string[]) {
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const matches = useMemo(() => findMatches(texts, query), [texts, query]);
  const bySegment = useMemo(() => {
    const grouped = new Map<number, Match[]>();
    for (const match of matches) {
      const group = grouped.get(match.segment);
      if (group) group.push(match);
      else grouped.set(match.segment, [match]);
    }
    return grouped;
  }, [matches]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        input.current?.focus();
        input.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => setCurrent(0), [query]);
  const total = matches.length;
  const focused = matches[Math.min(current, total - 1)] as Match | undefined;
  const turns = useRef(new Map<number, HTMLElement>());
  useEffect(() => {
    if (focused) turns.current.get(focused.segment)?.scrollIntoView?.({ block: "center" });
  }, [focused]);
  const turnRef = (position: number) => (element: HTMLElement | null) => {
    if (element) turns.current.set(position, element);
    else turns.current.delete(position);
  };
  return {
    input,
    query,
    total,
    matches,
    bySegment,
    focused,
    turnRef,
    setQuery,
    move: (step: 1 | -1) =>
      setCurrent((value) => (total === 0 ? 0 : (value + step + total) % total)),
  };
}
