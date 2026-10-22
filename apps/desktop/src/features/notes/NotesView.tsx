import type { AppView as AppSnapshot } from "../../state/app-view";
import { useEffect } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { NotesSnapshot } from "../../bridge/generated/NotesSnapshot";
import { Button } from "../../design/Button";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { NoteDetail } from "./NoteDetail";
import { NoteReadFailure } from "./NoteReadFailure";
import { NotesIndexMessage, NotesIndexStatus } from "./NotesIndexStatus";
import { NotesList } from "./NotesList";
import { NotesRefreshButton } from "./NotesRefreshButton";
import { useNoteArrows } from "./use-note-arrows";
import styles from "./NotesView.module.css";

interface NotesViewProps {
  error: string | null;
  onRecord: () => void;
  send: (command: AppCommand) => Promise<unknown>;
  snapshot: AppSnapshot;
}

function useNotesMidnightRefresh(send: (command: AppCommand) => Promise<unknown>) {
  useEffect(() => {
    let timer: number;
    const schedule = () => {
      const now = new Date();
      // The browser only schedules the wake at its next local midnight. The
      // app-core clock and zone decide the labels when NotesRefresh reads.
      const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
      timer = window.setTimeout(() => {
        void send({ command: "notes_refresh" });
        schedule();
      }, next.getTime() - now.getTime());
    };
    schedule();
    return () => window.clearTimeout(timer);
  }, [send]);
}

/** The note the route names is read when it is not the one read already;
 * the client reads it again when the hub says it changed. */
function useNoteRoute(snapshot: AppSnapshot, send: (command: AppCommand) => Promise<unknown>) {
  const routeRecordingId = decodeURIComponent(
    window.location.hash.match(/^#\/notes\/([^/]+)$/)?.[1] ?? "",
  );
  const readMatches = snapshot.notes?.open_read?.recording_id === routeRecordingId;
  useEffect(() => {
    if (routeRecordingId !== "" && !readMatches) {
      void send({ command: "notes_open", recording_id: routeRecordingId });
    }
  }, [readMatches, routeRecordingId, send]);
  return { routeRecordingId, readMatches };
}

function openNote(note: NoteListItem, send: (command: AppCommand) => Promise<unknown>) {
  window.location.hash = `#/notes/${encodeURIComponent(note.recording_id)}`;
  void send({ command: "notes_open", recording_id: note.recording_id });
}

function listTitle(notes: NotesSnapshot): string | undefined {
  return notes.list.find((note) => note.recording_id === notes.open?.recording_id)?.title;
}

/** Only a failed read shows a failure. */
function unreadNoteView(
  read: NotesSnapshot["open_read"],
  recordingId: string,
  send: (command: AppCommand) => Promise<unknown>,
) {
  if (read?.state !== "failed") return null;
  return <NoteReadFailure recordingId={recordingId} send={send} />;
}

function noteRouteView({
  error,
  notes,
  readMatches,
  routeRecordingId,
  send,
}: {
  error: string | null;
  notes: NotesSnapshot;
  readMatches: boolean;
  routeRecordingId: string;
  send: (command: AppCommand) => Promise<unknown>;
}) {
  const read = notes.open_read;
  if (routeRecordingId && !readMatches) {
    return (
      <section className={styles.empty}>
        <h1>{es.shell.notes}</h1>
        <p role="status">{es.notes.loadingNote}</p>
      </section>
    );
  }
  const unread = routeRecordingId ? unreadNoteView(read, routeRecordingId, send) : null;
  if (unread) return unread;
  if (routeRecordingId && read?.state === "loading") {
    return (
      <section className={styles.empty}>
        <h1>{es.shell.notes}</h1>
        <p role="status">{es.notes.loadingNote}</p>
      </section>
    );
  }
  if (
    notes.open &&
    (!routeRecordingId ||
      (read?.state === "available" && notes.open.recording_id === routeRecordingId))
  ) {
    return (
      <NoteDetail
        error={error}
        library={notes.library}
        note={notes.open}
        send={send}
        series={listTitle(notes) ?? notes.open.title}
      />
    );
  }
  return null;
}

export default function NotesView({ error, onRecord, send, snapshot }: NotesViewProps) {
  useNotesMidnightRefresh(send);
  const notes = snapshot.notes ?? { list: [], open: null };
  const { routeRecordingId, readMatches } = useNoteRoute(snapshot, send);
  const routeView = noteRouteView({
    error,
    notes,
    readMatches,
    routeRecordingId,
    send,
  });
  useNoteArrows(notes.list, routeRecordingId || (notes.open?.recording_id ?? ""), (note) =>
    openNote(note, send),
  );
  if (routeView) return routeView;
  if (notes.list.length > 0) {
    return (
      <>
        <ViewError error={error} />
        <NotesIndexStatus index={notes.index} />
        <NotesList
          more={notes.index?.more === true}
          notes={notes.list}
          onOpen={(note) => openNote(note, send)}
          send={send}
        />
      </>
    );
  }
  if (notes.index && notes.index.state !== "available")
    return <NotesIndexMessage index={notes.index} send={send} />;
  return (
    <section className={styles.empty}>
      <ViewError error={error} />
      <header className={styles.listHeader}>
        <h1>{es.shell.notes}</h1>
        <NotesRefreshButton send={send} />
      </header>
      <div className={styles.emptyBody}>
        <h2>{es.notes.empty}</h2>
        <p>{es.notes.emptyBody}</p>
        <Button onClick={onRecord} size={36} variant="accent">
          {es.meeting.recordNow}
        </Button>
      </div>
    </section>
  );
}
