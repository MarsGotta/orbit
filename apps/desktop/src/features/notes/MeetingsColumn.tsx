import { useCopy } from "../../i18n/CopyProvider";
import type { AppCommand } from "../../bridge/generated";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { NotesSnapshot } from "../../bridge/generated/NotesSnapshot";
import { Button } from "../../design/Button";
import { useFeedback } from "../../design/feedback";
import { Icon } from "../../design/Icon";
import { Loading, Spinner } from "../../design/Spinner";
import { Skeleton } from "../../design/Skeleton";
import { ListColumn } from "../../app/shell-slots";

import type { AppView } from "../../state/app-view";
import { noteRowState } from "./note-row";
import { dayGroups, groupFor, rowTime } from "./notes-format";
import { openMeeting, routeRecordingId } from "./note-route";
import { sent } from "./send";
import styles from "./MeetingsColumn.module.css";

type Send = (command: AppCommand) => Promise<unknown>;

function Row({ note, open, send }: { note: NoteListItem; open: boolean; send: Send }) {
  const es = useCopy();
  const line = noteRowState(note);
  return (
    <a
      aria-current={open ? "page" : undefined}
      className={styles.row}
      href={`#/notes/${encodeURIComponent(note.recording_id)}`}
      onClick={(event) => {
        event.preventDefault();
        openMeeting(note.recording_id, send);
      }}
    >
      <span className={styles.rowHead}>
        <span className={styles.rowTitle}>{note.title}</span>
        <time dateTime={new Date(note.started_at_ms).toISOString()}>{rowTime(note)}</time>
      </span>
      {note.recorded_on && (
        <span className={styles.rowLine}>
          <span className={styles.rowText} title={es.notes.recordedOn(note.recorded_on.name)}>
            {es.notes.recordedOn(note.recorded_on.name)}
          </span>
        </span>
      )}
      <span className={styles.rowLine} data-tone={line.tone}>
        {line.tone === "working" && <Spinner size={12} />}
        {(line.tone === "failed" || line.tone === "live") && (
          <span aria-hidden="true" className={styles.dot} />
        )}
        <span className={styles.rowText}>{line.text}</span>
      </span>
    </a>
  );
}

/** «Leyendo tus reuniones del hub…» with rows in the shape of what is coming. */
function ListLoading() {
  const es = useCopy();
  return (
    <div aria-busy="true" className={styles.loading}>
      <Loading size={12}>{es.notes.loadingIndex}</Loading>
      {["70%", "56%", "64%", "48%"].map((width, index) => (
        <div className={styles.loadingRow} key={index}>
          <Skeleton height={11} widths={[width]} />
          <Skeleton height={9} widths={[`${Number.parseInt(width, 10) / 1.8}%`]} />
        </div>
      ))}
    </div>
  );
}

function IndexMessage({ notes, send }: { notes: NotesSnapshot; send: Send }) {
  const es = useCopy();
  const index = notes.index;
  if (notes.list.length > 0) return null;
  if (index === undefined || index.state === "loading") return <ListLoading />;
  if (index.state === "failed") {
    return (
      <div className={styles.message} role="alert">
        <p>{es.notes.indexFailed}</p>
        <Button onClick={() => void send({ command: "notes_refresh" })} size={28}>
          {es.notes.retry}
        </Button>
      </div>
    );
  }
  if (index.state === "needs_hub") {
    return (
      <p className={styles.message} role="status">
        {es.notes.needsHub}
      </p>
    );
  }
  return null;
}

function RefreshButton({ send }: { send: Send }) {
  const es = useCopy();
  const { feedback, track } = useFeedback();
  const pending = feedback === "pending";
  return (
    <button
      aria-busy={pending || undefined}
      aria-label={es.notes.refresh}
      className={styles.refresh}
      onClick={() => {
        if (!pending) track(sent(send, { command: "notes_refresh" }));
      }}
      title={es.notes.refresh}
      type="button"
    >
      {pending ? <Spinner size={14} /> : <Icon name="refresh" size={16} />}
    </button>
  );
}

function MoreButton({ send }: { send: Send }) {
  const es = useCopy();
  const { feedback, track } = useFeedback();
  const pending = feedback === "pending";
  return (
    <Button
      aria-busy={pending || undefined}
      className={styles.more}
      onClick={() => {
        if (!pending) track(sent(send, { command: "notes_more" }));
      }}
      size={32}
      variant="outline"
    >
      {pending ? (
        <>
          <Spinner size={12} />
          {es.notes.loadingMore}
        </>
      ) : (
        es.notes.loadMore
      )}
    </Button>
  );
}

/**
 * The 272 px column of Reuniones, in the window's list column: the meetings by day, one line each, the
 * open one marked. «Cargar más» reads one more page of the hub's (fifty
 * meetings) when it has more.
 */
export function MeetingsColumn({ send, snapshot }: { send: Send; snapshot: AppView }) {
  const es = useCopy();
  const notes = snapshot.notes ?? { list: [], open: null };
  const routed = routeRecordingId();
  const openId = routed !== "" ? routed : (notes.open?.recording_id ?? "");
  return (
    <ListColumn
      actions={<RefreshButton send={send} />}
      label={es.notes.listLabel}
      title={es.notes.section}
    >
      <IndexMessage notes={notes} send={send} />
      {dayGroups().map(([group, label]) => {
        const rows = notes.list.filter((note) => groupFor(note) === group);
        if (rows.length === 0) return null;
        return (
          <section className={styles.group} key={group}>
            <h3>{label}</h3>
            {rows.map((note) => (
              <Row
                key={note.recording_id}
                note={note}
                open={note.recording_id === openId}
                send={send}
              />
            ))}
          </section>
        );
      })}
      {notes.index?.more === true && <MoreButton send={send} />}
    </ListColumn>
  );
}
