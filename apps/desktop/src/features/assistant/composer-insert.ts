import { useEffect } from "react";

type Insert = (text: string) => void;

let composer: Insert | null = null;

/**
 * Puts `text` at the end of the composer's draft without sending it
 * («Responder a esto»). Nothing happens when no composer is open.
 */
export function insertIntoComposer(text: string): void {
  composer?.(text);
}

/** The open composer receives what `insertIntoComposer` puts in it. */
export function useComposerInsert(insert: Insert): void {
  useEffect(() => {
    composer = insert;
    return () => {
      if (composer === insert) composer = null;
    };
  }, [insert]);
}
