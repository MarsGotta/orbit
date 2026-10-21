import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { useSpanish } from "../../i18n/AssistantCopy";
import { formatClock, formatDay, formatDuration } from "./notes-format";
import { isRecording } from "./note-progress";
import { DictionaryPicker } from "./DictionaryPicker";
import { LanguagePicker } from "./LanguagePicker";
import { SeriesAssign } from "./SeriesAssign";
import { TemplatePicker } from "./TemplatePicker";
import styles from "./NoteScreen.module.css";

type Send = (command: AppCommand) => void;

/**
 * The meeting's title, the one the list of Notes names it by (Mars,
 * 2026-10-17): one title in both places, never the one the note was written
 * with. Editing it renames the meeting on the hub, everywhere (SIM-52) when
 * she leaves the field or presses Enter; empty gives it back its own. A
 * recording the hub does not have whole yet cannot be renamed, and says so.
 */
function NoteTitle({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const [title, setTitle] = useState(note.title);
  const sent = useRef(note.title);
  useEffect(() => {
    // The hub's answer, or another change: what she sees is the hub's again.
    setTitle(note.title);
    sent.current = note.title;
  }, [note.title]);
  const notOnHub = note.progress.upload.state !== "done" ? es.noteScreen.titleNotOnHub : undefined;
  const commit = () => {
    const value = title.trim();
    // Empty: the hub gives the meeting back its own title.
    if (value === "") setTitle(note.title);
    if (value === sent.current.trim()) return;
    sent.current = value;
    send({ command: "meeting_rename", recording_id: note.recording_id, title: value });
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

function DataLine({ note }: { note: OpenNoteSnapshot }) {
  const parts = [
    formatDay(note.started_at_ms),
    `${formatClock(note.started_at_ms)} – ${formatClock(note.ended_at_ms)}`,
    formatDuration(Math.max(0, note.ended_at_ms - note.started_at_ms)),
  ];
  if (note.event) parts.push(note.event.title);
  return <p className={styles.dataLine}>{parts.join(" · ")}</p>;
}

function rewrite(note: OpenNoteSnapshot, send: Send) {
  send({
    command: "note_regenerate",
    recording_id: note.recording_id,
    template_id: note.summary?.template_id ?? "general",
  });
}

/** Mars changed the note the hub wrote: writing it again sets that aside. */
function editedByMars(note: OpenNoteSnapshot): boolean {
  return note.summary?.sections.some((section) => section.edited) ?? false;
}

/** Title, short summary, the meeting's data and what can be done with it. */
export function NoteHeader({
  library,
  note,
  onAsk,
  send,
}: {
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  onAsk: () => void;
  send: Send;
}) {
  const es = useSpanish();
  // While recording there is no note to write again yet: the controls stay
  // in place, disabled, and say why.
  const blocked = isRecording(note.progress) ? es.noteScreen.whileRecording : undefined;
  return (
    <header className={styles.head}>
      <NoteTitle key={note.recording_id} note={note} send={send} />
      {note.summary && <p className={styles.shortSummary}>{note.summary.short_summary}</p>}
      <DataLine note={note} />
      <div className={styles.headActions}>
        <Button onClick={onAsk} size={28} variant="accent">
          {es.noteScreen.askDeimos}
        </Button>
        <TemplatePicker blocked={blocked} library={library} note={note} send={send} />
        <LanguagePicker note={note} send={send} />
        <DictionaryPicker library={library} note={note} send={send} />
        <SeriesAssign library={library} note={note} send={send} />
        <span title={blocked}>
          <Button
            disabled={blocked !== undefined}
            onClick={() => rewrite(note, send)}
            size={28}
            variant="outline"
          >
            {es.noteScreen.rewrite}
          </Button>
        </span>
        {blocked === undefined && editedByMars(note) && (
          <span className={styles.templateState}>{es.noteScreen.rewriteSetsAside}</span>
        )}
      </div>
    </header>
  );
}
