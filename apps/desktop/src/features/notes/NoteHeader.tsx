import { useEffect, useRef, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Spinner } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
import { es } from "../../i18n/es";
import { MeetingMenu } from "./MeetingMenu";
import { isRecording } from "./note-progress";
import { NoteSettingsMenu } from "./NoteSettingsMenu";
import { formatClock, formatDay, formatLength } from "./notes-format";
import type { Send } from "./send";
import type { Rewriting } from "./use-rewriting";
import styles from "./Note.module.css";

/**
 * The meeting's title, the one the list names it by (decided 2026-10-17):
 * one title in both places, never the one the note was written with. Editing
 * it renames the meeting on the hub, everywhere, when the user leaves the
 * field or presses Enter; empty gives it back its own. A recording the hub
 * does not have whole yet cannot be renamed, and says so.
 */
function NoteTitle({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const [title, setTitle] = useState(note.title);
  const sentTitle = useRef(note.title);
  useEffect(() => {
    // The hub's answer, or another change: what the user sees is the hub's again.
    setTitle(note.title);
    sentTitle.current = note.title;
  }, [note.title]);
  const notOnHub = note.progress.upload.state !== "done" ? es.noteScreen.titleNotOnHub : undefined;
  const commit = () => {
    const value = title.trim();
    // Empty: the hub gives the meeting back its own title.
    if (value === "") setTitle(note.title);
    if (value === sentTitle.current.trim()) return;
    sentTitle.current = value;
    void send({ command: "meeting_rename", recording_id: note.recording_id, title: value });
  };
  return (
    <>
      <h1 className={styles.title}>
        <input
          aria-label={es.noteScreen.titleLabel}
          className={styles.titleInput}
          disabled={notOnHub !== undefined}
          onBlur={commit}
          onChange={(event) => setTitle(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape") setTitle(note.title);
          }}
          title={notOnHub}
          value={title}
        />
      </h1>
      {notOnHub && <p className={styles.dataLine}>{notOnHub}</p>}
    </>
  );
}

/** «Mié 14 oct · 11:00 – 11:47 · 47 min · Bruno, Celia y tú». */
function dataLine(note: OpenNoteSnapshot, row?: NoteListItem): string {
  const day =
    row?.group === "today"
      ? es.notes.today
      : row?.group === "yesterday"
        ? es.notes.yesterday
        : formatDay(note.started_at_ms);
  const parts = [
    day,
    `${formatClock(note.started_at_ms)} – ${formatClock(note.ended_at_ms)}`,
    formatLength(note.ended_at_ms - note.started_at_ms),
  ];
  const others = (note.event?.attendees ?? [])
    .filter((guest) => !guest.is_self)
    .map((guest) => (guest.name || guest.email).split(/\s+/)[0] ?? guest.email);
  if (others.length > 0) parts.push(es.noteScreen.withPeople(others));
  if (note.recorded_on) parts.push(es.notes.recordedOn(note.recorded_on.name));
  return parts.join(" · ");
}

/** Title, short summary, the meeting's data and what can be done with it. */
export function NoteHeader({
  library,
  note,
  onAsk,
  rewriting,
  row,
  send,
}: {
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  onAsk: () => void;
  /** «Rehacer resumen» lives in the Resumen's toolbar (annex, change 7); its status shows here. */
  rewriting: Rewriting;
  row?: NoteListItem;
  send: Send;
}) {
  const copy = useSpanish();
  // While recording there is no note to write again yet: the control stays
  // in place, disabled, and says why.
  const blocked = isRecording(note.progress) ? es.noteScreen.whileRecording : undefined;
  return (
    <header className={styles.head}>
      <NoteTitle key={note.recording_id} note={note} send={send} />
      {note.summary && <p className={styles.shortSummary}>{note.summary.short_summary}</p>}
      <p className={styles.dataLine}>{dataLine(note, row)}</p>
      {note.capture === "open" && note.recorded_on && (
        <p className={styles.dataLine}>{es.notes.pendingAudioOn(note.recorded_on.name)}</p>
      )}
      <div className={styles.headActions}>
        <Button onClick={onAsk} size={36} variant="outline">
          <span className={styles.askIcon}>
            <Icon name="ask" size={14} />
          </span>
          {copy.noteScreen.askDeimos}
        </Button>
        <NoteSettingsMenu
          blocked={blocked}
          library={library}
          note={note}
          onRewrite={rewriting.start}
          send={send}
        />
        {/* The meeting being recorded is thrown away from its card instead. */}
        {blocked === undefined && <MeetingMenu note={note} send={send} />}
      </div>
      <HeadStatus note={note} template={rewriting.template} />
    </header>
  );
}

/**
 * What the hub is doing with the note now: writing it, or transcribing it
 * again; and what this Mac changed that the hub has not taken yet.
 */
function HeadStatus({ note, template }: { note: OpenNoteSnapshot; template: string | null }) {
  const again = note.transcript.retranscribing;
  return (
    <>
      {(note.pending_changes?.length ?? 0) > 0 && (
        <p className={styles.headStatus} role="status">
          {es.noteScreen.changesOnMac}
        </p>
      )}
      {template !== null && (
        <p className={styles.headStatus} role="status">
          <Spinner size={14} />
          {es.noteScreen.rewriting(template)}
        </p>
      )}
      {again && (
        <p className={styles.headStatus} role="status">
          <Spinner size={14} />
          {es.noteScreen.retranscribing(again.done, again.total)}
        </p>
      )}
    </>
  );
}
