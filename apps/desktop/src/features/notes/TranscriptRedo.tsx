import { useEffect, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { ConfirmCard } from "../../design/ConfirmCard";
import { Icon } from "../../design/Icon";
import { ProgressCard } from "../../design/ProgressCard";
import { es } from "../../i18n/es";
import { meetingDictionaries, nameOf, takesLists } from "./dictionary-choice";
import { formatLength } from "./notes-format";
import { sent, type Send } from "./send";
import styles from "./Redo.module.css";

/** Where transcribing the meeting again stands, as this view has seen it. */
export type RedoPhase = "idle" | "queued" | "running" | "done" | "failed";

/**
 * «Rehacer transcripción» from the press to its end (annex, change 7). The
 * hub runs the pass and says how it goes (`transcript.final_pass`); the end
 * is told only to whoever saw it under way here, never for a pass of long ago.
 */
export function useRetranscribe(note: OpenNoteSnapshot, send: Send) {
  const [asked, setAsked] = useState(false);
  const [seen, setSeen] = useState(false);
  const [closed, setClosed] = useState(false);
  const pass = note.transcript.final_pass;
  const active = pass?.state === "queued" || pass?.state === "running";
  useEffect(() => {
    if (!active) return;
    setSeen(true);
    setAsked(false);
    setClosed(false);
  }, [active]);
  let phase: RedoPhase = "idle";
  if (active) phase = pass.state === "queued" ? "queued" : "running";
  else if (asked) phase = "queued";
  else if (pass?.state === "failed" && !closed) phase = "failed";
  else if (seen && pass?.state === "completed" && !closed) phase = "done";
  const start = () => {
    setAsked(true);
    setClosed(false);
    sent(send, { command: "transcript_retranscribe", recording_id: note.recording_id }).catch(() =>
      setAsked(false),
    );
  };
  return { phase, start, close: () => setClosed(true) };
}

/** What the question says it will use: the length, the language and the dictionaries. */
function question(note: OpenNoteSnapshot, library: NoteLibrarySnapshot | null | undefined) {
  const language = note.transcript.language;
  const names = (meetingDictionaries(note) ?? []).map((id) => nameOf(library, id));
  return es.notes.retranscribeBody(
    formatLength(note.ended_at_ms - note.started_at_ms),
    language === null ? null : (es.notes.languagesInSentence[language] ?? language),
    names,
  );
}

/**
 * The toolbar's «Rehacer transcripción», with its anchored question first. It
 * is not offered before the recording has closed, and waits while a pass is
 * under way.
 */
export function RedoTranscriptButton({
  library,
  note,
  phase,
  onConfirm,
}: {
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  phase: RedoPhase;
  onConfirm: () => void;
}) {
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null);
  const [asking, setAsking] = useState(false);
  if (note.capture !== "closed") return null;
  return (
    <>
      <Button
        aria-expanded={asking}
        disabled={phase === "queued" || phase === "running"}
        onClick={() => setAsking(!asking)}
        ref={setAnchor}
        size={36}
        variant="outline"
      >
        <Icon name="refresh" size={14} />
        {es.notes.retranscribe}
      </Button>
      {asking && (
        <ConfirmCard
          anchor={anchor}
          confirmLabel={es.notes.retranscribe}
          note={takesLists(note) ? es.notes.retranscribeKeeps : es.notes.retranscribeKeepsOlderHub}
          onClose={() => setAsking(false)}
          onConfirm={() => {
            setAsking(false);
            onConfirm();
          }}
          title={es.notes.retranscribeTitle}
        >
          {question(note, library)}
        </ConfirmCard>
      )}
    </>
  );
}

/** «unos 3 min» and the bar, from the hub's own measure of the pass. */
function measured(note: OpenNoteSnapshot) {
  const progress = note.transcript.final_pass?.progress;
  if (!progress) return {};
  return {
    percent: progress.total_ms > 0 ? (progress.done_ms / progress.total_ms) * 100 : undefined,
    remaining:
      progress.remaining_ms === undefined ? undefined : es.notes.remaining(progress.remaining_ms),
  };
}

/**
 * Under the toolbar: the pass in the queue or under way, its end with
 * «Rehacer resumen», or why it failed, the earlier transcript kept; and, in
 * a quiet line, the spans where it kept the live draft, which was better.
 */
export function RedoStatus({
  note,
  phase,
  onClose,
  onRedoSummary,
}: {
  note: OpenNoteSnapshot;
  phase: RedoPhase;
  onClose: () => void;
  onRedoSummary: () => void;
}) {
  const pass = note.transcript.final_pass;
  const unmatched = note.transcript.unmatched_speaker_names ?? 0;
  const kept = pass?.state === "completed" ? (pass.kept_draft?.spans ?? 0) : 0;
  return (
    <>
      {(phase === "queued" || phase === "running" || phase === "failed") && (
        <div className={styles.card}>
          {phase === "queued" && <ProgressCard>{es.notes.finalPassQueued}</ProgressCard>}
          {phase === "running" && (
            <ProgressCard {...measured(note)}>
              {es.notes.finalPassRunning}
              <span className={styles.aside}>{es.notes.finalPassAside}</span>
            </ProgressCard>
          )}
          {phase === "failed" && (
            <ProgressCard failed onClose={onClose}>
              {es.notes.finalPassFailed(pass?.error ?? null, pass?.timeout_ms)}
            </ProgressCard>
          )}
        </div>
      )}
      {phase === "done" && (
        <p className={styles.done} role="status">
          <span aria-hidden="true" className={styles.doneMark}>
            ✓
          </span>
          <span className={styles.doneText}>
            {note.summary ? es.notes.retranscribedRedo : es.notes.retranscribed}
          </span>
          {note.summary && (
            <Button onClick={onRedoSummary} size={28} variant="ghost">
              {es.noteScreen.redoSummary}
            </Button>
          )}
        </p>
      )}
      {kept > 0 && (
        <p className={styles.quiet} role="status">
          {es.notes.keptDraft(kept)}
        </p>
      )}
      {unmatched > 0 && (
        <p className={styles.quiet} role="status">
          {es.notes.unmatchedSpeakerNames(unmatched)}
        </p>
      )}
    </>
  );
}
