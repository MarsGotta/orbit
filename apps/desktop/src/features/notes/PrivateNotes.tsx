import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Spinner } from "../../design/Spinner";
import { es } from "../../i18n/es";
import { MarkdownEditor } from "../editor/MarkdownEditor";
import type { NoteWriting } from "./note-writing";
import {
  MAX_PRIVATE_NOTES_CHARS,
  type SaveState,
  usePrivateNotesDraft,
} from "./private-notes-draft";
import type { Send } from "./send";
import styles from "./Note.module.css";

/** «Guardando…», «Guardado en este Mac · solo lo ves tú», or why it was not kept. */
function SaveStatus({
  error,
  recovered,
  state,
}: {
  error: string | null;
  recovered: boolean;
  state: SaveState;
}) {
  // Brought back from an earlier run that closed or fell before saving it.
  if (recovered && state === "saving") return <>{es.controls.draftRecovered}</>;
  if (state === "saving") {
    return (
      <>
        <Spinner size={12} />
        {es.editor.saving}
      </>
    );
  }
  if (state === "refused") {
    return (
      <span className={styles.saveRefused}>
        {error === null ? es.editor.notSavedUnknown : es.editor.notSaved(error)}
      </span>
    );
  }
  return <>{es.editor.savedHere}</>;
}

/**
 * The user's private notes: the Markdown editor, its text kept encrypted on
 * this Mac by the daemon and sent with the note request.
 */
export function PrivateNotes({
  error,
  note,
  send,
  writing,
}: {
  /** Why the last order of the note was refused: said when it was a save. */
  error: string | null;
  note: OpenNoteSnapshot;
  send: Send;
  writing?: NoteWriting;
}) {
  const { draft, flush, recovered, state, update } = usePrivateNotesDraft(
    note.recording_id,
    note.private_notes,
    send,
  );
  return (
    <div className={styles.privateNotes}>
      <MarkdownEditor
        assistant={writing?.assistant}
        help
        label={es.noteScreen.privateLabel}
        maxChars={MAX_PRIVATE_NOTES_CHARS}
        notes={writing?.notes}
        onBlur={flush}
        onChange={update}
        people={writing?.people}
        status={<SaveStatus error={error} recovered={recovered} state={state} />}
        value={draft}
      />
    </div>
  );
}
