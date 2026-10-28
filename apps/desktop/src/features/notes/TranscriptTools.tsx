import { useCopy } from "../../i18n/CopyProvider";
import type { ReactNode } from "react";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Icon } from "../../design/Icon";

import { CopyButton } from "./copy-button";
import { transcriptText } from "./notes-format";
import type { useTranscriptSearch } from "./use-transcript-search";
import styles from "./Note.module.css";

/** The search, «Copiar» and «Rehacer transcripción», above the turns. */
export function Toolbar({
  note,
  search,
  redo,
}: {
  note: OpenNoteSnapshot;
  search: ReturnType<typeof useTranscriptSearch>;
  /** «Rehacer transcripción» and its question. */
  redo: ReactNode;
}) {
  const es = useCopy();
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
      {redo}
    </div>
  );
}

/** The parts of the meeting whose audio never reached the hub: said with when
 *  and for how long, since the transcript has nothing there. */
export function MissingAudio({ note }: { note: OpenNoteSnapshot }) {
  const es = useCopy();
  return (
    <>
      {(note.transcript.missing_audio ?? []).map((hole) => (
        <p className={styles.quiet} key={`${hole.track}-${hole.at_ms ?? "unknown"}`} role="status">
          {es.notes.missingAudio(hole.track, hole.at_ms, hole.duration_ms)}
        </p>
      ))}
    </>
  );
}

/** «Silenciar»: each stretch where the microphone was muted, in one calm line. */
export function MutedMicrophone({ note }: { note: OpenNoteSnapshot }) {
  const es = useCopy();
  return (
    <>
      {(note.transcript.muted_microphone ?? []).map((span) => (
        <p className={styles.quiet} key={span.start_ms}>
          {es.notes.mutedMicrophone(span.start_ms, span.end_ms)}
        </p>
      ))}
    </>
  );
}
