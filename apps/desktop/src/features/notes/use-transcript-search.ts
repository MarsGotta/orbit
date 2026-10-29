import { useEffect, useMemo, useRef, useState } from "react";
import { type Match, findMatches } from "./transcript-search";
import { useWindowActions } from "../../state/window-actions";

/**
 * The transcript's search, always at hand: the query, its matches and the one
 * in focus. ⌘F puts the cursor in it; ↵ and ⇧↵ move between matches, and only
 * a match the user moved to scrolls into view (nothing moves on opening).
 */
export function useTranscriptSearch(texts: string[], available = true) {
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const matches = useMemo(() => findMatches(texts, query), [texts, query]);
  useEffect(() => {
    if (!available) return;
    const find = () => {
      input.current?.focus();
      input.current?.select();
    };
    useWindowActions.setState({ findTranscript: find });
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        input.current?.focus();
        input.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (useWindowActions.getState().findTranscript === find)
        useWindowActions.setState({ findTranscript: null });
    };
  }, [available]);
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
    focused,
    turnRef,
    setQuery,
    move: (step: 1 | -1) =>
      setCurrent((value) => (total === 0 ? 0 : (value + step + total) % total)),
  };
}
