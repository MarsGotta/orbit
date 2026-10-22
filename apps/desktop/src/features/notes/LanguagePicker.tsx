import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { useHeldChoice } from "../../state/use-held-choice";
import { languageName } from "./notes-format";
import styles from "./NoteScreen.module.css";

/** The languages a meeting can be picked in, in display order. */
const LANGUAGES = ["es", "en", "ca", "fr", "pt", "de", "it"];

/**
 * The meeting's language, the one the hub's document says. Another one
 * transcribes the meeting again in it and writes its note again;
 * meanwhile it says how far that got. Not offered before the hub has the
 * meeting: there is no language to show.
 */
export function LanguagePicker({
  note,
  send,
}: {
  note: OpenNoteSnapshot;
  send: (command: AppCommand) => unknown;
}) {
  // The language chosen stays shown until the hub has the meeting in it.
  const language = useHeldChoice(note.transcript.language, send);
  const current = language.shown;
  if (current == null) return null;
  const options = LANGUAGES.includes(current) ? LANGUAGES : [...LANGUAGES, current];
  const repeating = note.transcript.retranscribing;
  return (
    <>
      <label className={styles.templatePicker}>
        <span>{es.noteScreen.language}</span>
        <select
          onChange={(event) =>
            language.choose(event.target.value, {
              command: "meeting_language",
              recording_id: note.recording_id,
              language: event.target.value,
            })
          }
          value={current}
        >
          {options.map((code) => (
            <option key={code} value={code}>
              {languageName(code)}
            </option>
          ))}
        </select>
      </label>
      {repeating && (
        <span className={styles.templateState} role="status">
          {es.noteScreen.retranscribing(repeating.done, repeating.total)}
        </span>
      )}
    </>
  );
}
