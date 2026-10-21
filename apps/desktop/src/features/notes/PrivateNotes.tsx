import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { NoteEditor } from "./NoteEditor";
import { usePrivateNotesDraft } from "./private-notes-draft";
import styles from "./NoteScreen.module.css";

/**
 * Mars's private notes (P9-04): a rich editor whose markdown the daemon
 * keeps encrypted and sends with the note request.
 */
export function PrivateNotes({
  note,
  send,
}: {
  note: OpenNoteSnapshot;
  send: (command: AppCommand) => void;
}) {
  const [value, update] = usePrivateNotesDraft(note.recording_id, note.private_notes, send);
  return (
    <section className={styles.privateNotes}>
      <p className={styles.help}>{es.notes.privateHelp}</p>
      <NoteEditor
        label={es.notes.privateLabel}
        markdown={value}
        onChange={update}
        placeholder={es.notes.privatePlaceholder}
      />
    </section>
  );
}
