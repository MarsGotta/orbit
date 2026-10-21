import type { AppView as AppSnapshot } from "../state/app-view";
import type { NoteListItem } from "../bridge/generated/NoteListItem";
import { noteRowState, type RowTone } from "../features/notes/note-row";
import { DAY_GROUPS, groupFor } from "../features/notes/notes-format";
import { es } from "../i18n/es";
import type { ContextPanelData, Send } from "./ContextPanel";

function rowLine(note: NoteListItem): { subtitle: string; tone: RowTone } {
  const state = noteRowState(note);
  return { subtitle: state.text, tone: state.tone };
}

/** The notes list beside the open note: by day, each with its line. */
export function notesData(snapshot?: AppSnapshot, send?: Send): ContextPanelData {
  const notes = snapshot?.notes;
  if (notes) {
    return {
      title: es.shell.notes,
      count: String(notes.list.length),
      groups: DAY_GROUPS.flatMap(([group, label]) => {
        const rows = notes.list
          .filter((note) => groupFor(note) === group)
          .map((note) => ({
            active: notes.open?.recording_id === note.recording_id,
            initial: note.title.charAt(0).toUpperCase(),
            meta: new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short" }).format(
              note.started_at_ms,
            ),
            ...rowLine(note),
            title: note.title,
            onSelect: () => {
              window.location.hash = `#/notes/${encodeURIComponent(note.recording_id)}`;
              send?.({ command: "notes_open", recording_id: note.recording_id });
            },
          }));
        return rows.length > 0 ? [{ label, rows }] : [];
      }),
    };
  }
  return { title: es.shell.notes, count: "0", groups: [] };
}
