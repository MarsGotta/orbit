import type { AppCommand } from "../../bridge/generated";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import { es } from "../../i18n/es";
import { DAY_GROUPS, formatDate, groupFor } from "./notes-format";
import { noteRowState } from "./note-row";
import { NotesRefreshButton } from "./NotesRefreshButton";
import styles from "./NotesView.module.css";

function NoteRow({ note, onOpen }: { note: NoteListItem; onOpen: (note: NoteListItem) => void }) {
  const line = noteRowState(note);
  return (
    <button className={styles.listRow} onClick={() => onOpen(note)} type="button">
      <span className={styles.listInitial}>{note.title.charAt(0).toUpperCase()}</span>
      <span className={styles.listCopy}>
        <strong>{note.title}</strong>
        <small data-tone={line.tone}>{line.text}</small>
      </span>
      <time dateTime={new Date(note.started_at_ms).toISOString()}>
        {formatDate(note.started_at_ms)}
      </time>
    </button>
  );
}

/** The list, by day; «Cargar más» reads one more page of the hub's (fifty
 * meetings) when it has more. */
export function NotesList({
  more,
  notes,
  onOpen,
  send,
}: {
  more: boolean;
  notes: NoteListItem[];
  onOpen: (note: NoteListItem) => void;
  send: (command: AppCommand) => Promise<void>;
}) {
  const visible = notes;
  return (
    <section className={styles.listView}>
      <header className={styles.listHeader}>
        <h1>{es.shell.notes}</h1>
        <NotesRefreshButton send={send} />
      </header>
      {DAY_GROUPS.map(([group, label]) => {
        const rows = visible.filter((note) => groupFor(note) === group);
        if (rows.length === 0) return null;
        return (
          <section className={styles.listGroup} key={group}>
            <h2>{label}</h2>
            {rows.map((note) => (
              <NoteRow key={note.recording_id} note={note} onOpen={onOpen} />
            ))}
          </section>
        );
      })}
      {more && (
        <button onClick={() => void send({ command: "notes_more" })} type="button">
          {es.notes.loadMore}
        </button>
      )}
    </section>
  );
}
