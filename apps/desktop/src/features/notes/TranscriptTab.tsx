import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { SpeakerRenameScope } from "../../bridge/generated/SpeakerRenameScope";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";
import { es } from "../../i18n/es";
import { CorrectionDialog } from "./CorrectionDialog";
import { templateOf } from "./note-progress";
import { sent, type Send } from "./send";
import { SpeakerControl } from "./SpeakerControl";
import { TranscriptSegment } from "./TranscriptSegment";
import { FinalPassStatus, Toolbar } from "./TranscriptTools";
import { useTranscriptSearch } from "./use-transcript-search";
import { useWordSelection } from "./use-word-selection";
import styles from "./Note.module.css";

/** More turns are on their way: the transcript is not complete yet. */
function arriving(note: OpenNoteSnapshot): boolean {
  return note.transcript.pending_chunks > 0 || note.progress.transcript.state === "active";
}

/** «✓ Cambiado en 3 turnos. La nota usará el cambio cuando rehagas el resumen. Rehacer resumen». */
function DoneBand({ note, send, text }: { note: OpenNoteSnapshot; send: Send; text: string }) {
  return (
    <p className={styles.doneBand} role="status">
      <span aria-hidden="true" className={styles.doneMark}>
        ✓
      </span>
      <span className={styles.doneText}>{text}</span>
      {note.summary !== null && (
        <Button
          onClick={() =>
            void send({
              command: "note_regenerate",
              recording_id: note.recording_id,
              template_id: templateOf(note),
            })
          }
          size={28}
          variant="ghost"
        >
          {es.noteScreen.redoSummary}
        </Button>
      )}
    </p>
  );
}

/** «Pulsa un nombre…», or «La transcripción va apareciendo…» while it fills. */
function Hint({ note }: { note: OpenNoteSnapshot }) {
  return (
    <p className={styles.quiet}>
      {arriving(note) ? es.notes.transcriptArriving : es.notes.transcriptHelp}
    </p>
  );
}

function TranscriptState({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  if (note.transcript.error != null && note.transcript.error !== "") {
    return (
      <div className={styles.inlineFailure} role="alert">
        <span>{es.notes.transcriptFailed}</span>
        <Button
          onClick={() => void send({ command: "notes_open", recording_id: note.recording_id })}
          size={28}
        >
          {es.notes.retry}
        </Button>
      </div>
    );
  }
  if (note.transcript.turns.length === 0 && !arriving(note))
    return <p className={styles.quiet}>{es.notes.noTranscript}</p>;
  return null;
}

/** Under a turn with a word selected: «Corregir «Orvit»». */
function CorrectButton({ text, onCorrect }: { text: string; onCorrect: (text: string) => void }) {
  return (
    <Button
      className={styles.correctButton}
      onClick={() => onCorrect(text)}
      size={28}
      variant="outline"
    >
      <Icon name="edit" size={14} />
      {es.notes.correct(text)}
    </Button>
  );
}

/** The end of a transcript still filling: skeleton lines. */
function Arriving() {
  return (
    <div aria-label={es.notes.moreTranscript} className={styles.arriving} role="img">
      <Skeleton height={10} widths={["100px", "80%"]} />
    </div>
  );
}

interface TranscriptTabProps {
  note: OpenNoteSnapshot;
  send: Send;
  library?: NoteLibrarySnapshot | null;
}

/** The speaker menu that is open, the window that corrects a word, and the band after either. */
function useTranscriptChanges(note: OpenNoteSnapshot, send: Send, clearWords: () => void) {
  const [menu, setMenu] = useState<number | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [correcting, setCorrecting] = useState<string | null>(null);
  useEffect(() => setDone(null), [note.recording_id]);
  const written = note.summary !== null;
  const band = useCallback(
    (first: string) => setDone(written ? `${first} ${es.notes.usesOnRewrite}` : first),
    [written],
  );
  const rename =
    (turn: NoteTurnSnapshot) => (name: string, scope: SpeakerRenameScope, count: number) =>
      sent(send, { command: "speaker_rename", speaker_key: turn.speaker_key, name, scope }).then(
        () => {
          setMenu(null);
          band(es.notes.renamed(count));
        },
      );
  const saved = useCallback(
    (wrong: string, correct: string) => {
      setCorrecting(null);
      clearWords();
      band(es.notes.corrected(wrong, correct));
    },
    [band, clearWords],
  );
  return { menu, setMenu, done, correcting, setCorrecting, rename, saved };
}

/**
 * The transcript as the hub computed it: a search, «Copiar», «Rehacer
 * transcripción», each turn's speaker opening «¿Quién es…?», and «Corregir»
 * on a selected word. It opens where it starts, never at its end (proposal 23).
 */
export function TranscriptTab({ note, send, library }: TranscriptTabProps) {
  const turns = note.transcript.turns;
  const search = useTranscriptSearch(useMemo(() => turns.map((turn) => turn.text), [turns]));
  const container = useRef<HTMLDivElement>(null);
  const words = useWordSelection(container);
  const changes = useTranscriptChanges(note, send, words.clear);
  const { menu, setMenu, done, correcting, setCorrecting, rename, saved } = changes;
  return (
    <div className={styles.transcript}>
      {!arriving(note) && <Toolbar note={note} search={search} send={send} />}
      <Hint note={note} />
      <FinalPassStatus note={note} />
      {done !== null && <DoneBand note={note} send={send} text={done} />}
      <TranscriptState note={note} send={send} />
      <div
        className={styles.turns}
        onKeyUp={words.capture}
        onMouseUp={words.capture}
        ref={container}
      >
        {note.transcript.turns.map((turn, position) => (
          <TranscriptSegment
            after={
              words.selected?.turn === turn.index && (
                <CorrectButton onCorrect={setCorrecting} text={words.selected.text} />
              )
            }
            focused={search.focused}
            key={turn.index}
            matches={search.matches.filter((match) => match.segment === position)}
            ref={search.turnRef(position)}
            turn={turn}
            who={
              <SpeakerControl
                note={note}
                onApply={rename(turn)}
                open={menu === turn.index}
                setOpen={setMenu}
                speaker={note.speakers.find((speaker) => speaker.key === turn.speaker_key)}
                turn={turn}
              />
            }
          />
        ))}
        {arriving(note) && <Arriving />}
      </div>
      {correcting !== null && (
        <CorrectionDialog
          library={library}
          note={note}
          onClose={() => setCorrecting(null)}
          onSaved={saved}
          send={send}
          wrong={correcting}
        />
      )}
    </div>
  );
}
