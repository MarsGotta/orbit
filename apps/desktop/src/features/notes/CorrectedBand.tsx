import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useRef, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";

import {
  conflictsOf,
  nameOf,
  pending,
  refusalOf,
  released,
  takesLists,
  undoOrder,
} from "./dictionary-choice";
import { sent, type Send } from "./send";
import type { Correction } from "./WordMenu";
import styles from "./Correction.module.css";

/**
 * «Deshacer» from the press until the hub's read shows the correction gone
 * from every dictionary, or says why not. It is not confirmed: it undoes.
 */
function useUndo(
  correction: Correction,
  library: NoteLibrarySnapshot | null | undefined,
  send: Send,
  onUndone: () => void,
) {
  const [asked, setAsked] = useState(false);
  const { ids, wrong, correct, target } = correction;
  const onItsWay = asked && pending(library, target);
  const refused = asked && !onItsWay ? refusalOf(library, target) : undefined;
  const gone =
    asked && !onItsWay && refused === undefined && released(library, ids, wrong, correct);
  const told = useRef(false);
  useEffect(() => {
    if (!gone || told.current) return;
    told.current = true;
    onUndone();
  });
  const undo = () => {
    setAsked(true);
    sent(send, {
      command: "note_library",
      request: { action: "dictionary", request: undoOrder(ids, wrong, correct) },
    }).catch(() => setAsked(false));
  };
  return { undo, waiting: asked && refused === undefined && !gone, refused };
}

/** What the hub warned of: another dictionary of the meeting hears the word otherwise. */
function Conflicts({ library, target }: { library?: NoteLibrarySnapshot | null; target: string }) {
  const es = useCopy();
  return conflictsOf(library, target).map((conflict, index) => (
    <p className={styles.conflict} key={index}>
      {es.notes.conflict(
        conflict.other_name,
        conflict.wrong,
        conflict.other_correct,
        conflict.applied_dictionary_id === null
          ? null
          : nameOf(library, conflict.applied_dictionary_id),
      )}
    </p>
  ));
}

/**
 * «✓ «Orvit» ahora es «Orbit» en 3 sitios y está en Nubaria y Hoteles
 * Ribera.» with «Deshacer» and «Rehacer resumen», and what the hub warned
 * of: another dictionary of the meeting that hears the word otherwise.
 */
export function CorrectedBand({
  correction,
  library,
  note,
  send,
  onUndone,
  onRedoSummary,
}: {
  correction: Correction;
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  send: Send;
  onUndone: () => void;
  onRedoSummary: () => void;
}) {
  const es = useCopy();
  const { undo, waiting, refused } = useUndo(correction, library, send, onUndone);
  const { wrong, correct, places, names } = correction;
  return (
    <div className={styles.band} role="status">
      <p className={styles.bandRow}>
        <span aria-hidden="true" className={styles.bandMark}>
          ✓
        </span>
        <span className={styles.bandText}>
          {es.notes.corrected(wrong, correct, places, names)}
          {note.summary && ` ${es.notes.usesOnRewrite}`}
        </span>
        {/* An older hub has no «Deshacer» of one correction: only the whole entry goes. */}
        {takesLists(note) && (
          <Button
            feedback={waiting ? "pending" : "idle"}
            onClick={undo}
            pendingLabel={es.controls.saving}
            size={28}
            variant="ghost"
          >
            {es.notes.undo}
          </Button>
        )}
        {note.summary && (
          <Button onClick={onRedoSummary} size={28} variant="ghost">
            {es.noteScreen.redoSummary}
          </Button>
        )}
      </p>
      <Conflicts library={library} target={correction.target} />
      {refused !== undefined && (
        <p className={styles.refused} role="alert">
          {refused}
        </p>
      )}
    </div>
  );
}

/** «Corrección deshecha. «Orvit» ya no está en Nubaria y Hoteles Ribera.» */
export function UndoneBand({ correction }: { correction: Correction }) {
  const es = useCopy();
  return (
    <p className={styles.undone} role="status">
      {es.notes.undone(correction.wrong, correction.names)}
    </p>
  );
}
