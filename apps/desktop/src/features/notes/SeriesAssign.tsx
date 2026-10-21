import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { refusalText, seriesRefusals } from "../../app/library-refusal";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import styles from "./NoteScreen.module.css";

type Send = (command: AppCommand) => void;

/** «Usar siempre en esta serie»: the note's template writes the series' notes. */
export function SeriesAssign({
  library,
  note,
  send,
}: {
  library: NoteLibrarySnapshot | null | undefined;
  note: OpenNoteSnapshot;
  send: Send;
}) {
  const series = note.series;
  if (series == null || note.summary === null) return null;
  return (
    <>
      <AssignButton library={library} note={note} send={send} />
      {seriesRefusals(library?.refusals, series.calendar_item_id).map((refusal, index) => (
        <span className={styles.notice} key={index} role="alert">
          {refusalText(refusal)}
        </span>
      ))}
    </>
  );
}

function AssignButton({
  library,
  note,
  send,
}: {
  library: NoteLibrarySnapshot | null | undefined;
  note: OpenNoteSnapshot;
  send: Send;
}) {
  const series = note.series;
  const template = note.summary?.template_id;
  if (series == null || template === undefined) return null;
  if (
    series.assigned.state === "known" &&
    (series.assigned.template_id ?? "general") === template
  ) {
    const name = library?.items.find((item) => item.id === template)?.name;
    return <span className={styles.templateState}>{es.library.seriesUses(name ?? template)}</span>;
  }
  return (
    <Button
      onClick={() =>
        send({
          command: "note_library",
          request: {
            action: "assign",
            calendar_item_id: series.calendar_item_id,
            template_id: template,
          },
        })
      }
      size={28}
      variant="ghost"
    >
      {es.library.useAlways}
    </Button>
  );
}
