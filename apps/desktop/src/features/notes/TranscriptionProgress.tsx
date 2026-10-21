import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import styles from "./NotesView.module.css";

/** Where the transcript stands: being transcribed, partly failed (with
 * «Reintentar»), or unreadable. Counts and causes stay in the log (SIM-24). */
export function TranscriptionProgress({
  note,
  send,
}: {
  note: OpenNoteSnapshot;
  send: (command: AppCommand) => void;
}) {
  const transcript = note.transcript;
  if (transcript.error != null && transcript.error !== "") {
    return <TranscriptReadFailure note={note} send={send} />;
  }
  if (transcript.failed_groups > 0) {
    return (
      <div className={styles.failed} role="status">
        <span>{es.notes.failedGroups}</span>
        <button
          onClick={() => send({ command: "transcript_retry", recording_id: note.recording_id })}
          type="button"
        >
          {es.notes.retry}
        </button>
      </div>
    );
  }
  if (transcript.pending_chunks === 0) return null;
  return (
    <div className={styles.progress} role="status">
      <span>{es.notes.transcribing}</span>
    </div>
  );
}

function TranscriptReadFailure({
  note,
  send,
}: {
  note: OpenNoteSnapshot;
  send: (command: AppCommand) => void;
}) {
  return (
    <div className={styles.failed} role="alert">
      <span>{es.notes.transcriptFailed}</span>
      <button
        onClick={() => send({ command: "notes_open", recording_id: note.recording_id })}
        type="button"
      >
        {es.notes.retry}
      </button>
    </div>
  );
}
