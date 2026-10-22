import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { useHeldChoice } from "../../state/use-held-choice";
import styles from "./NoteScreen.module.css";

/**
 * The hub's templates. Choosing another one writes the note again with it.
 * A list never read or whose read failed says so; it is never shown as «no
 * templates».
 */
export function TemplatePicker({
  blocked,
  library,
  note,
  send,
}: {
  /** Why the template cannot change now (while recording), when it cannot. */
  blocked?: string;
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  send: (command: AppCommand) => unknown;
}) {
  // The template chosen stays shown until the note is written again with it.
  const template = useHeldChoice(note.summary?.template_id ?? "general", send);
  // Templates not read yet, or not readable without the hub, say
  // nothing; a failed read says so.
  if (
    library == null ||
    library.templates.state === "unknown" ||
    library.templates.state === "needs_hub"
  )
    return null;
  if (library.read === "failed" || library.templates.state === "failed") {
    return (
      <span className={styles.templateState} role="status">
        {es.noteScreen.templatesFailed}
      </span>
    );
  }
  return (
    <label className={styles.templatePicker} title={blocked}>
      <span>{es.noteScreen.template}</span>
      <select
        disabled={blocked !== undefined}
        onChange={(event) =>
          template.choose(event.target.value, {
            command: "note_regenerate",
            recording_id: note.recording_id,
            template_id: event.target.value,
          })
        }
        value={template.shown}
      >
        {library.items.map((template) => (
          <option
            key={template.id}
            title={es.noteScreen.templateChange(template.name)}
            value={template.id}
          >
            {template.name}
          </option>
        ))}
      </select>
    </label>
  );
}
