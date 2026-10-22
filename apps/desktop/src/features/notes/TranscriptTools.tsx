import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { es } from "../../i18n/es";
import { CopyButton } from "./copy-button";
import { transcriptText } from "./notes-format";
import type { Send } from "./send";
import type { useTranscriptSearch } from "./use-transcript-search";
import styles from "./Note.module.css";

/** The search, «Copiar» and «Rehacer transcripción», above the turns. */
export function Toolbar({
  note,
  search,
  send,
}: {
  note: OpenNoteSnapshot;
  search: ReturnType<typeof useTranscriptSearch>;
  send: Send;
}) {
  const offerAgain =
    note.capture === "closed" &&
    note.transcript.final_pass?.state !== "queued" &&
    note.transcript.final_pass?.state !== "running";
  return (
    <div className={styles.transcriptTools}>
      <div className={styles.searchField} role="search">
        <Icon name="search" size={14} />
        <input
          aria-label={es.notes.searchLabel}
          onChange={(event) => search.setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              search.move(event.shiftKey ? -1 : 1);
            } else if (event.key === "Escape" && search.query !== "") {
              event.preventDefault();
              search.setQuery("");
            }
          }}
          placeholder={es.notes.searchLabel}
          ref={search.input}
          type="search"
          value={search.query}
        />
        <span aria-live="polite" className={styles.searchCount}>
          {search.query.trim() === "" ? "" : es.notes.searchResults(search.total)}
        </span>
      </div>
      <CopyButton label={es.notes.copy} size={36} text={() => transcriptText(note)} />
      {offerAgain && (
        <Button
          onClick={() =>
            void send({ command: "transcript_retranscribe", recording_id: note.recording_id })
          }
          size={36}
          variant="ghost"
        >
          {es.notes.retranscribe}
        </Button>
      )}
    </div>
  );
}

/** Where a second pass over the transcript stands. */
export function FinalPassStatus({ note }: { note: OpenNoteSnapshot }) {
  const pass = note.transcript.final_pass;
  const unmatched = note.transcript.unmatched_speaker_names ?? 0;
  return (
    <>
      {pass?.state === "queued" && (
        <p className={styles.quiet} role="status">
          {es.notes.finalPassQueued}
        </p>
      )}
      {pass?.state === "running" && (
        <p className={styles.quiet} role="status">
          {es.notes.finalPassRunning}
        </p>
      )}
      {pass?.state === "failed" && (
        <p className={styles.menuRefusal} role="alert">
          {es.notes.finalPassFailed(pass.error, pass.timeout_ms)}
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
