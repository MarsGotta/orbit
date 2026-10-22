import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { NoteEditor } from "./NoteEditor";
import { usePrivateNotesDraft } from "./private-notes-draft";
import type { Send } from "./send";
import styles from "./Note.module.css";

/**
 * The user's private notes: a rich editor whose markdown the daemon keeps
 * encrypted on this Mac and sends with the note request.
 */
export function PrivateNotes({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const [value, update] = usePrivateNotesDraft(note.recording_id, note.private_notes, (command) => {
    void send(command);
  });
  return (
    <div className={styles.privateNotes}>
      <p className={styles.quiet}>{es.noteScreen.privateHelp}</p>
      <div className={styles.privateBox}>
        <NoteEditor
          label={es.noteScreen.privateLabel}
          markdown={value}
          onChange={update}
          placeholder={es.noteScreen.privatePlaceholder}
        />
      </div>
    </div>
  );
}
