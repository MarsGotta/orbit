import { useEffect, useMemo, useRef, useState } from "react";
import { type Match, findMatches } from "./transcript-search";

/** ⌘F in the transcript: open, query, matches and the one in focus. */
export function useTranscriptSearch(texts: string[]) {
  const [open, setOpen] = useState(false);
  // Each ⌘F, also with the bar already open, puts the cursor back in it.
  const [summons, setSummons] = useState(0);
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState(0);
  const matches = useMemo(() => (open ? findMatches(texts, query) : []), [open, texts, query]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        setOpen(true);
        setSummons((value) => value + 1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => setCurrent(0), [query]);
  const total = matches.length;
  const focused = matches[Math.min(current, total - 1)] as Match | undefined;
  // The turn holding the match in focus scrolls into view.
  const turns = useRef(new Map<number, HTMLElement>());
  useEffect(() => {
    if (focused) turns.current.get(focused.segment)?.scrollIntoView?.({ block: "center" });
  }, [focused]);
  const turnRef = (position: number) => (element: HTMLElement | null) => {
    if (element) turns.current.set(position, element);
    else turns.current.delete(position);
  };
  return {
    open,
    summons,
    query,
    total,
    current: total === 0 ? 0 : Math.min(current, total - 1),
    matches,
    focused,
    turnRef,
    setQuery,
    move: (step: 1 | -1) =>
      setCurrent((value) => (total === 0 ? 0 : (value + step + total) % total)),
    close: () => {
      setOpen(false);
      setQuery("");
    },
  };
}
