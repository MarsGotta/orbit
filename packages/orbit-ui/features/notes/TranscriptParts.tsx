import { useCopy } from "../../i18n/CopyProvider";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";

import type { Send } from "./send";
import styles from "./Note.module.css";

// The small parts of the transcript around its turns.

/** More turns are on their way: the transcript is not complete yet. */
export function arriving(note: OpenNoteSnapshot): boolean {
  return note.transcript.pending_chunks > 0 || note.progress.transcript.state === "active";
}

/** «✓ Cambiado en 3 turnos. La nota usará el cambio cuando rehagas el resumen. Rehacer resumen». */
export function DoneBand({
  note,
  text,
  onRedoSummary,
}: {
  note: OpenNoteSnapshot;
  text: string;
  onRedoSummary: () => void;
}) {
  const es = useCopy();
  return (
    <p className={styles.doneBand} role="status">
      <span aria-hidden="true" className={styles.doneMark}>
        ✓
      </span>
      <span className={styles.doneText}>{text}</span>
      {note.summary !== null && (
        <Button onClick={onRedoSummary} size={28} variant="ghost">
          {es.noteScreen.redoSummary}
        </Button>
      )}
    </p>
  );
}

/** «Pulsa un nombre…», or «La transcripción va apareciendo…» while it fills. */
export function Hint({ note }: { note: OpenNoteSnapshot }) {
  const es = useCopy();
  return (
    <p className={styles.quiet}>
      {arriving(note) ? es.notes.transcriptArriving : es.notes.transcriptHelp}
    </p>
  );
}

export function TranscriptState({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const es = useCopy();
  if (note.transcript.error != null && note.transcript.error !== "") {
    return (
      <div className={styles.inlineFailure} role="alert">
        <span>{es.notes.transcriptFailed}</span>
        <Button
          onClick={() => void send({ command: "notes_open", recording_id: note.recording_id })}
          size={28}
        >
          {es.notes.retry}
        </Button>
      </div>
    );
  }
  if (note.transcript.turns.length === 0 && !arriving(note))
    return <p className={styles.quiet}>{es.notes.noTranscript}</p>;
  return null;
}

/** Under a turn with text selected: «Corregir «Orvit»», the keyboard's way to the same menu. */
export function CorrectButton({
  text,
  onCorrect,
}: {
  text: string;
  onCorrect: (text: string, anchor: HTMLElement) => void;
}) {
  const es = useCopy();
  return (
    <Button
      className={styles.correctButton}
      onClick={(event) => onCorrect(text, event.currentTarget)}
      size={28}
      variant="outline"
    >
      <Icon name="edit" size={14} />
      {es.notes.correct(text)}
    </Button>
  );
}

/** The end of a transcript still filling: skeleton lines. */
export function Arriving() {
  const es = useCopy();
  return (
    <div aria-label={es.notes.moreTranscript} className={styles.arriving} role="img">
      <Skeleton height={10} widths={["100px", "80%"]} />
    </div>
  );
}
