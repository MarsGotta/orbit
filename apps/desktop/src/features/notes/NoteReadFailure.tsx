import type { AppCommand } from "../../bridge/generated";
import { es } from "../../i18n/es";
import styles from "./NotesView.module.css";

export function NoteReadFailure({
  recordingId,
  send,
}: {
  recordingId: string;
  send: (command: AppCommand) => Promise<unknown>;
}) {
  return (
    <section className={styles.empty}>
      <h1>{es.shell.notes}</h1>
      <p role="alert">{es.notes.noteReadFailed}</p>
      <button
        type="button"
        onClick={() => void send({ command: "notes_open", recording_id: recordingId })}
      >
        {es.notes.retry}
      </button>
    </section>
  );
}
