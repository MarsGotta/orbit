import { useCopy } from "../../i18n/CopyProvider";
import { useCallback, useEffect, useState } from "react";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { SpeakerRenameScope } from "../../bridge/generated/SpeakerRenameScope";

import { sent, type Send } from "./send";
import type { Fix } from "./transcript-fix";
import type { Correction } from "./WordMenu";
import type { Span } from "./word-forms";

/** The word being corrected: pressed in a turn, or selected with the keyboard. */
export interface Picked {
  wrong: string;
  turn?: number;
  span?: Span;
  /** The control it hangs from when no word is drawn: «Corregir «…»». */
  button?: HTMLElement;
}

/** The menus open, the bands after them, and the correction laid over the turns. */
export function useTranscriptChanges(note: OpenNoteSnapshot, send: Send, clearWords: () => void) {
  const es = useCopy();
  const [menu, setMenu] = useState<number | null>(null);
  const [renamed, setRenamed] = useState<string | null>(null);
  const [picked, setPicked] = useState<Picked | null>(null);
  const [fix, setFix] = useState<Fix | null>(null);
  const [undone, setUndone] = useState<Correction | null>(null);
  useEffect(() => {
    setRenamed(null);
    setFix(null);
    setUndone(null);
  }, [note.recording_id]);
  const written = note.summary !== null;
  const rename =
    (turn: NoteTurnSnapshot) => (name: string, scope: SpeakerRenameScope, count: number) =>
      sent(send, { command: "speaker_rename", speaker_key: turn.speaker_key, name, scope }).then(
        () => {
          setMenu(null);
          const first = es.notes.renamed(count);
          setRenamed(written ? `${first} ${es.notes.usesOnRewrite}` : first);
        },
      );
  const close = useCallback(() => setPicked(null), []);
  const saved = (next: Fix) => {
    setPicked(null);
    clearWords();
    setRenamed(null);
    setUndone(null);
    setFix(next);
  };
  const markUndone = () => {
    setUndone(fix?.correction ?? null);
    setFix(null);
  };
  return {
    menu,
    setMenu,
    renamed,
    picked,
    setPicked,
    close,
    fix,
    rename,
    saved,
    undone,
    markUndone,
  };
}

/** Whether a press on `span` of a turn is a press on the word already open: it closes it. */
export function same(picked: Picked | null, turn: number, span: Span): boolean {
  return picked?.turn === turn && picked.span?.start === span.start && picked.span.end === span.end;
}
