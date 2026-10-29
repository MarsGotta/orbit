import { type RefObject, useMemo, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { BeingRedone } from "../../design/ProgressCard";
import { CorrectedBand, UndoneBand } from "./CorrectedBand";
import { templateOf } from "./note-progress";
import type { Send } from "./send";
import { SpeakerControl } from "./SpeakerControl";
import { fixOf, shownText } from "./transcript-fix";
import {
  Arriving,
  CorrectButton,
  DoneBand,
  Hint,
  TranscriptState,
  arriving,
} from "./TranscriptParts";
import { RedoStatus, RedoTranscriptButton, useRetranscribe } from "./TranscriptRedo";
import { TranscriptSegment } from "./TranscriptSegment";
import { MissingAudio, MutedMicrophone, Toolbar } from "./TranscriptTools";
import { TurnText } from "./TurnText";
import { same, useTranscriptChanges } from "./use-transcript-changes";
import { useTranscriptSearch } from "./use-transcript-search";
import { useWordSelection } from "./use-word-selection";
import { WordMenu } from "./WordMenu";
import styles from "./Note.module.css";

interface TranscriptTabProps {
  note: OpenNoteSnapshot;
  send: Send;
  library?: NoteLibrarySnapshot | null;
  /** «Rehacer resumen» from a band: writes the note again and shows the Resumen. */
  onRedoSummary?: () => void;
}

interface TurnsProps {
  note: OpenNoteSnapshot;
  changes: ReturnType<typeof useTranscriptChanges>;
  words: ReturnType<typeof useWordSelection>;
  container: RefObject<HTMLDivElement | null>;
  texts: string[];
  search: ReturnType<typeof useTranscriptSearch>;
  busy: boolean;
  onAnchor: (element: HTMLElement | null) => void;
}

/**
 * The turns: who speaks (a button that opens «¿Quién es…?»), and what was
 * said, every word opening «Corregir «…»». Dimmed and out of reach while the
 * meeting is transcribed again.
 */
function Turns({ note, changes, words, container, texts, search, busy, onAnchor }: TurnsProps) {
  const { menu, setMenu, picked, setPicked, fix, rename } = changes;
  return (
    <BeingRedone active={busy}>
      <div
        className={styles.turns}
        onKeyUp={words.capture}
        onMouseUp={words.capture}
        ref={container}
      >
        {note.transcript.turns.map((turn, position) => {
          const text = texts[position] ?? turn.text;
          const ours = fix !== null && fix.after.get(turn.index) === text;
          const selected = words.selected?.turn === turn.index ? words.selected.text : null;
          const correct = (wrong: string, button: HTMLElement) => setPicked({ wrong, button });
          return (
            <TranscriptSegment
              after={selected !== null && <CorrectButton onCorrect={correct} text={selected} />}
              key={turn.index}
              ref={search.turnRef(position)}
              said={
                <TurnText
                  anchorRef={onAnchor}
                  fixed={ours ? (fix.places.get(turn.index) ?? []) : []}
                  focused={search.focused}
                  matches={search.matches.filter((match) => match.segment === position)}
                  onPick={(span) =>
                    setPicked(
                      same(picked, turn.index, span)
                        ? null
                        : { wrong: text.slice(span.start, span.end), turn: turn.index, span },
                    )
                  }
                  picked={picked?.turn === turn.index ? (picked.span ?? null) : null}
                  text={text}
                />
              }
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
          );
        })}
        {arriving(note) && <Arriving />}
      </div>
    </BeingRedone>
  );
}

/** The note written again with its own template, when no Resumen is at hand to show it. */
function rewrite(note: OpenNoteSnapshot): AppCommand {
  return {
    command: "note_regenerate",
    recording_id: note.recording_id,
    template_id: templateOf(note),
  };
}

/** After a correction: «✓ … ahora es …» with «Deshacer», or «Corrección deshecha». */
function CorrectionBands({
  changes,
  library,
  note,
  onRedoSummary,
  send,
}: {
  changes: ReturnType<typeof useTranscriptChanges>;
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  onRedoSummary: () => void;
  send: Send;
}) {
  return (
    <>
      {changes.fix && (
        <CorrectedBand
          correction={changes.fix.correction}
          library={library}
          note={note}
          onRedoSummary={onRedoSummary}
          onUndone={changes.markUndone}
          send={send}
        />
      )}
      {changes.undone && <UndoneBand correction={changes.undone} />}
    </>
  );
}

/**
 * The transcript as the hub computed it: a search, «Copiar», «Rehacer
 * transcripción», each turn's speaker opening «¿Quién es…?», and every word
 * opening «Corregir «…»» (annex, changes 5 and 7). It opens where it starts,
 * never at its end (proposal 23).
 */
export function TranscriptTab({ note, send, library, onRedoSummary }: TranscriptTabProps) {
  const container = useRef<HTMLDivElement>(null);
  const words = useWordSelection(container);
  const changes = useTranscriptChanges(note, send, words.clear);
  const { renamed, picked, fix } = changes;
  const turns = note.transcript.turns;
  const texts = useMemo(() => turns.map((turn) => shownText(turn, fix)), [turns, fix]);
  const search = useTranscriptSearch(texts, !arriving(note));
  const redo = useRetranscribe(note, send);
  const [wordAnchor, setWordAnchor] = useState<HTMLElement | null>(null);
  const redoSummary = onRedoSummary ?? (() => void send(rewrite(note)));
  const button = (
    <RedoTranscriptButton library={library} note={note} onConfirm={redo.start} phase={redo.phase} />
  );
  return (
    <div className={styles.transcript}>
      {!arriving(note) && <Toolbar note={note} redo={button} search={search} />}
      <RedoStatus note={note} onClose={redo.close} onRedoSummary={redoSummary} phase={redo.phase} />
      <MissingAudio note={note} />
      <MutedMicrophone note={note} />
      {renamed !== null && <DoneBand note={note} onRedoSummary={redoSummary} text={renamed} />}
      <CorrectionBands
        changes={changes}
        library={library}
        note={note}
        onRedoSummary={redoSummary}
        send={send}
      />
      <Hint note={note} />
      <TranscriptState note={note} send={send} />
      <Turns
        busy={redo.phase === "queued" || redo.phase === "running"}
        changes={changes}
        container={container}
        note={note}
        onAnchor={setWordAnchor}
        search={search}
        texts={texts}
        words={words}
      />
      {picked !== null && (
        <WordMenu
          anchor={picked.button ?? wordAnchor}
          key={`${picked.turn ?? "selected"}-${picked.span?.start ?? picked.wrong}`}
          library={library}
          note={note}
          onClose={changes.close}
          onSaved={(correction) => changes.saved(fixOf(library, correction))}
          send={send}
          wrong={picked.wrong}
        />
      )}
    </div>
  );
}
