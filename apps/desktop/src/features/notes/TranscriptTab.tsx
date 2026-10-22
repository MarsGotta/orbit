import { useEffect, useMemo, useRef, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import { TranscriptCorrection } from "./TranscriptCorrection";
import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { languageName, speakerName, transcriptText } from "./notes-format";
import { SpeakerMenuView } from "./SpeakerMenu";
import { SpeakerRenameDialog } from "./SpeakerRenameDialog";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import { TranscriptSearch } from "./TranscriptSearch";
import { TranscriptSegment } from "./TranscriptSegment";
import { useTranscriptSearch } from "./use-transcript-search";
import styles from "./NoteScreen.module.css";

type Send = (command: AppCommand) => void;

/**
 * Follows the end of the transcript while new turns arrive, only when the user
 * is already there: reading further up is never interrupted.
 */
function useFollowEnd(count: number) {
  const end = useRef<HTMLDivElement>(null);
  const atEnd = useRef(true);
  useEffect(() => {
    const target = end.current;
    if (!target || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => {
      atEnd.current = entries.some((entry) => entry.isIntersecting);
    });
    observer.observe(target);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (atEnd.current) end.current?.scrollIntoView?.({ block: "end" });
  }, [count]);
  return end;
}

function Tools({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  // The meeting's language, as the hub's document says it.
  const language = note.transcript.language;
  return (
    <div className={styles.toolbar}>
      {language != null && (
        <span>{`${es.notes.transcriptLanguage}: ${languageName(language)}`}</span>
      )}
      {note.capture === "closed" &&
        note.transcript.final_pass?.state !== "queued" &&
        note.transcript.final_pass?.state !== "running" && (
          <Button
            onClick={() =>
              send({ command: "transcript_retranscribe", recording_id: note.recording_id })
            }
            size={28}
            variant="ghost"
          >
            {es.notes.retranscribe}
          </Button>
        )}
      <Button
        onClick={() => void navigator.clipboard?.writeText(transcriptText(note))}
        size={28}
        variant="ghost"
      >
        {es.noteScreen.copyTranscript}
      </Button>
    </div>
  );
}

function FinalPassStatus({ note }: { note: OpenNoteSnapshot }) {
  return (
    <>
      {note.transcript.final_pass?.state === "queued" && (
        <p role="status">{es.notes.finalPassQueued}</p>
      )}
      {note.transcript.final_pass?.state === "running" && (
        <p role="status">{es.notes.finalPassRunning}</p>
      )}
      {note.transcript.final_pass?.state === "failed" && (
        <p role="alert">
          {es.notes.finalPassFailed(
            note.transcript.final_pass.error,
            note.transcript.final_pass.timeout_ms,
          )}
        </p>
      )}
      {(note.transcript.unmatched_speaker_names ?? 0) > 0 && (
        <p role="status">
          {es.notes.unmatchedSpeakerNames(note.transcript.unmatched_speaker_names ?? 0)}
        </p>
      )}
    </>
  );
}

interface SpeakerSelection {
  turn: NoteTurnSnapshot;
  stage: "select" | "add" | "scope";
  name: string;
}

function SpeakerSelectionView({
  note,
  send,
  selection,
  setSelection,
}: {
  note: OpenNoteSnapshot;
  send: Send;
  selection: SpeakerSelection;
  setSelection: React.Dispatch<React.SetStateAction<SpeakerSelection | null>>;
}) {
  const choose = (name: string) => setSelection({ ...selection, stage: "scope", name });
  if (selection.stage === "select")
    return (
      <SpeakerMenuView
        note={note}
        speakerKey={selection.turn.speaker_key}
        onChoose={choose}
        onAdd={(name) => setSelection({ ...selection, stage: "add", name })}
        onClose={() => setSelection((current) => (current?.stage === "select" ? null : current))}
      />
    );
  return (
    <SpeakerRenameDialog
      key={selection.stage}
      name={selection.name}
      speakerName={speakerName(note, selection.turn)}
      atMs={selection.turn.at_ms}
      adding={selection.stage === "add"}
      onChoose={choose}
      onClose={() => setSelection(null)}
      onApply={(scope) => {
        send({
          command: "speaker_rename",
          speaker_key: selection.turn.speaker_key,
          name: selection.name,
          scope,
        });
        setSelection(null);
      }}
    />
  );
}

function isEmpty(note: OpenNoteSnapshot): boolean {
  const transcript = note.transcript;
  return (
    transcript.turns.length === 0 &&
    transcript.failed_groups === 0 &&
    transcript.pending_chunks === 0 &&
    (transcript.error == null || transcript.error === "")
  );
}

/** The transcript as the hub computed it, searchable with ⌘F, with speaker
 * menus. While recording it grows as the hub transcribes. */
interface TranscriptTabProps {
  note: OpenNoteSnapshot;
  send: Send;
  library?: NoteLibrarySnapshot | null;
}

export function TranscriptTab({ note, send, library }: TranscriptTabProps) {
  const [selection, setSelection] = useState<SpeakerSelection | null>(null);
  const turns = note.transcript.turns;
  const texts = useMemo(() => turns.map((turn) => turn.text), [turns]);
  const search = useTranscriptSearch(texts);
  const end = useFollowEnd(turns.length);
  return (
    <section>
      {search.open && (
        <TranscriptSearch
          current={search.current}
          onClose={search.close}
          onMove={search.move}
          onQuery={search.setQuery}
          query={search.query}
          summons={search.summons}
          total={search.total}
        />
      )}
      <Tools note={note} send={send} />
      <FinalPassStatus note={note} />
      {note.summary && <p className={styles.emptyCopy}>{es.notes.speakerNoteHelp}</p>}
      {selection && (
        <SpeakerSelectionView
          note={note}
          send={send}
          selection={selection}
          setSelection={setSelection}
        />
      )}
      {isEmpty(note) && <p className={styles.emptyCopy}>{es.notes.noTranscript}</p>}
      <TranscriptCorrection note={note} library={library} send={send}>
        {turns.map((turn, position) => (
          <TranscriptSegment
            focused={search.focused}
            key={turn.index}
            matches={search.matches.filter((match) => match.segment === position)}
            note={note}
            onSpeaker={(turn) => setSelection({ turn, stage: "select", name: "" })}
            ref={search.turnRef(position)}
            turn={turn}
          />
        ))}
      </TranscriptCorrection>
      <div aria-hidden="true" className={styles.end} ref={end} />
    </section>
  );
}

export { TranscriptionProgress } from "./TranscriptionProgress";
