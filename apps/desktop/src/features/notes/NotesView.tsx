import { useCopy } from "../../i18n/CopyProvider";
import { useEffect } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { NotesSnapshot } from "../../bridge/generated/NotesSnapshot";
import { Button } from "../../design/Button";
import { EmptyState } from "../../design/EmptyState";
import { Loading } from "../../design/Spinner";
import { ViewError } from "../../design/ViewError";

import type { AppView } from "../../state/app-view";
import { MeetingsColumn } from "./MeetingsColumn";
import { NoteDetail } from "./NoteDetail";
import { NoteGone, NoteLoading, NoteReadFailure } from "./NoteReadStates";
import { openMeeting, routeRecordingId } from "./note-route";
import { noteWriting } from "./note-writing";
import { useNoteArrows } from "./use-note-arrows";
import styles from "./Note.module.css";

type Send = (command: AppCommand) => Promise<unknown>;

interface NotesViewProps {
  error: string | null;
  onRecord: () => void;
  send: Send;
  snapshot: AppView;
}

function useNotesMidnightRefresh(send: Send) {
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
function useNoteRoute(notes: NotesSnapshot, send: Send) {
  const routeId = routeRecordingId();
  const readMatches = notes.open_read?.recording_id === routeId;
  useEffect(() => {
    if (routeId !== "" && !readMatches) void send({ command: "notes_open", recording_id: routeId });
  }, [readMatches, routeId, send]);
  return { routeId, readMatches };
}

function IndexFailure({ send }: { send: Send }) {
  const es = useCopy();
  return (
    <div className={styles.centered} role="alert">
      <p className={styles.centeredText}>{es.notes.indexFailed}</p>
      <Button onClick={() => void send({ command: "notes_refresh" })} size={36} variant="accent">
        {es.notes.retry}
      </Button>
    </div>
  );
}

/** What the main area shows when no meeting is open: a prompt, or why the
 * list is empty. Never «Aún no hay…» before the list is read. */
function Unopened({
  notes,
  onRecord,
  send,
}: {
  notes: NotesSnapshot;
  onRecord: () => void;
  send: Send;
}) {
  const es = useCopy();
  const index = notes.index;
  if (notes.list.length > 0) {
    return (
      <div className={styles.centered}>
        <EmptyState icon="notes" title={es.notes.choose}>
          {es.notes.chooseBody}
        </EmptyState>
      </div>
    );
  }
  if (index === undefined || index.state === "loading") {
    return (
      <div aria-busy="true" className={styles.centered}>
        <Loading>{es.notes.loadingIndex}</Loading>
      </div>
    );
  }
  if (index.state === "failed") return <IndexFailure send={send} />;
  if (index.state === "needs_hub") {
    return (
      <div className={styles.centered}>
        <p className={styles.centeredText} role="status">
          {es.notes.needsHub}
        </p>
      </div>
    );
  }
  return (
    <div className={styles.centered}>
      <EmptyState
        action={
          <Button onClick={onRecord} size={36} variant="accent">
            <span aria-hidden="true" className={styles.recordDot} />
            {es.notes.record}
          </Button>
        }
        icon="notes"
        title={es.notes.empty}
      >
        {es.notes.emptyBody}
      </EmptyState>
    </div>
  );
}

function opened({
  error,
  notes,
  routeId,
  readMatches,
  send,
  snapshot,
}: {
  error: string | null;
  notes: NotesSnapshot;
  routeId: string;
  readMatches: boolean;
  send: Send;
  snapshot: AppView;
}) {
  const read = notes.open_read;
  if (routeId !== "") {
    if (!readMatches || read?.state === "loading") return <NoteLoading />;
    if (read?.state === "failed") return <NoteReadFailure recordingId={routeId} send={send} />;
    if (read?.state === "gone") return <NoteGone />;
  }
  const note = notes.open;
  if (!note || (routeId !== "" && note.recording_id !== routeId)) return null;
  // A meeting being deleted is left as soon as it was asked.
  if (routeId === "" && note.pending_changes?.includes("delete") === true) return null;
  return (
    <NoteDetail
      error={error}
      library={notes.library}
      note={note}
      row={notes.list.find((item) => item.recording_id === note.recording_id)}
      send={send}
      writing={noteWriting(snapshot, note, send)}
    />
  );
}

/** Reuniones: the list in the window's list column (`MeetingsColumn`), and
 * here the open meeting, or why there is none. */
export default function NotesView({ error, onRecord, send, snapshot }: NotesViewProps) {
  const es = useCopy();
  useNotesMidnightRefresh(send);
  const notes = snapshot.notes ?? { list: [], open: null };
  const { routeId, readMatches } = useNoteRoute(notes, send);
  useNoteArrows(notes.list, routeId || (notes.open?.recording_id ?? ""), (note) =>
    openMeeting(note.recording_id, send),
  );
  const note = opened({ error, notes, routeId, readMatches, send, snapshot });
  return (
    <section aria-label={es.notes.section} className={styles.view}>
      <MeetingsColumn send={send} snapshot={snapshot} />
      {note ?? (
        <>
          <ViewError error={error} />
          <Unopened notes={notes} onRecord={onRecord} send={send} />
        </>
      )}
    </section>
  );
}
