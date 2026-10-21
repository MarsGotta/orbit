import { useEffect, useRef } from "react";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import styles from "./NoteScreen.module.css";

function SearchCount({ current, query, total }: { current: number; query: string; total: number }) {
  return (
    <span aria-live="polite" className={styles.searchCount}>
      {query.trim() === ""
        ? ""
        : total === 0
          ? es.noteScreen.searchNone
          : es.noteScreen.searchCount(current + 1, total)}
    </span>
  );
}

function SearchMoves({ onClose, onMove }: { onClose: () => void; onMove: (step: 1 | -1) => void }) {
  return (
    <>
      <Button
        aria-label={es.noteScreen.searchPrevious}
        onClick={() => onMove(-1)}
        size={26}
        variant="ghost"
      >
        ↑
      </Button>
      <Button
        aria-label={es.noteScreen.searchNext}
        onClick={() => onMove(1)}
        size={26}
        variant="ghost"
      >
        ↓
      </Button>
      <Button aria-label={es.noteScreen.searchClose} onClick={onClose} size={26} variant="ghost">
        ×
      </Button>
    </>
  );
}

/** ⌘F over the transcript: the text, «n / total», ↵ / ⇧↵ to move, Esc. */
export function TranscriptSearch({
  current,
  onClose,
  onMove,
  onQuery,
  query,
  summons,
  total,
}: {
  current: number;
  onClose: () => void;
  onMove: (step: 1 | -1) => void;
  onQuery: (query: string) => void;
  query: string;
  /** Changes on every ⌘F: the input takes the focus again. */
  summons: number;
  total: number;
}) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), [summons]);
  return (
    <div className={styles.search} role="search">
      <input
        aria-label={es.noteScreen.searchLabel}
        onChange={(event) => onQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onMove(event.shiftKey ? -1 : 1);
          } else if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
        placeholder={es.noteScreen.searchPlaceholder}
        ref={input}
        value={query}
      />
      <SearchCount current={current} query={query} total={total} />
      <SearchMoves onClose={onClose} onMove={onMove} />
    </div>
  );
}
