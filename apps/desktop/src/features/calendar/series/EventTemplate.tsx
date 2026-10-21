import type { AppCommand } from "../../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import { refusalText, seriesRefusals } from "../../../app/library-refusal";
import { es } from "../../../i18n/es";
import styles from "./EventTemplate.module.css";

const GENERAL = "general";

/**
 * «Plantilla de la nota»: the template the notes of this event and its
 * series are written with. Choosing General removes the assignment. A list
 * not read yet says nothing (SIM-24); it is never shown as General.
 */
export function EventTemplate({
  calendarItemId,
  library,
  send,
}: {
  calendarItemId: string;
  library: NoteLibrarySnapshot | null | undefined;
  send: (command: AppCommand) => Promise<void> | void;
}) {
  return (
    <section aria-label={es.library.noteTemplate} className={styles.section}>
      <TemplateState calendarItemId={calendarItemId} library={library} send={send} />
    </section>
  );
}

function TemplateState({
  calendarItemId,
  library,
  send,
}: {
  calendarItemId: string;
  library: NoteLibrarySnapshot | null | undefined;
  send: (command: AppCommand) => Promise<void> | void;
}) {
  if (library?.templates.state === "failed") {
    return (
      <p className={styles.single} role="status">
        {es.library.templatesFailed}
      </p>
    );
  }
  if (library?.templates.state !== "available" || library.links.state !== "available") return null;
  const assigned =
    library.assignments.find((link) => link.calendar_item_id === calendarItemId)?.template_id ??
    GENERAL;
  return (
    <>
      <TemplateSelect
        assigned={assigned}
        calendarItemId={calendarItemId}
        library={library}
        send={send}
      />
      {seriesRefusals(library.refusals, calendarItemId).map((refusal, index) => (
        <p className={styles.alert} key={index} role="alert">
          {refusalText(refusal)}
        </p>
      ))}
    </>
  );
}

function TemplateSelect({
  assigned,
  calendarItemId,
  library,
  send,
}: {
  assigned: string;
  calendarItemId: string;
  library: NoteLibrarySnapshot;
  send: (command: AppCommand) => Promise<void> | void;
}) {
  return (
    <label className={styles.template} title={es.library.noteTemplateHint}>
      <span>{es.library.noteTemplate}</span>
      <select
        onChange={(change) =>
          void send({
            command: "note_library",
            request: {
              action: "assign",
              calendar_item_id: calendarItemId,
              template_id: change.target.value === GENERAL ? null : change.target.value,
            },
          })
        }
        value={assigned}
      >
        {library.items.map((template) => (
          <option key={template.id} value={template.id}>
            {template.name}
          </option>
        ))}
      </select>
    </label>
  );
}
