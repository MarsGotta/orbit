import { useEffect } from "react";

type Insert = (text: string) => void;

let composer: Insert | null = null;
/** What was put in while no composer was open: the next one to open takes it. */
let waiting: string | null = null;

/**
 * Puts `text` at the end of the composer's draft without sending it
 * («Responder a esto», «Preguntar a …» from a note). With no composer open,
 * the next composer to open receives it.
 */
export function insertIntoComposer(text: string): void {
  if (composer) composer(text);
  else waiting = waiting === null ? text : `${waiting}${text}`;
}

/** The open composer receives what `insertIntoComposer` puts in it. */
export function useComposerInsert(insert: Insert): void {
  useEffect(() => {
    composer = insert;
    if (waiting !== null) {
      const text = waiting;
      waiting = null;
      insert(text);
    }
    return () => {
      if (composer === insert) composer = null;
    };
  }, [insert]);
}
