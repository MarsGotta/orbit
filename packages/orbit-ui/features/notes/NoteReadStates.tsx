import { useCopy } from "../../i18n/CopyProvider";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { EmptyState } from "../../design/EmptyState";
import { Loading } from "../../design/Spinner";
import { Skeleton } from "../../design/Skeleton";

import styles from "./Note.module.css";

type Send = (command: AppCommand) => Promise<unknown>;

/** A note on its way: its shape, and what is being read. */
export function NoteLoading() {
  const es = useCopy();
  return (
    <div aria-busy="true" className={styles.page}>
      <div className={styles.reading}>
        <Skeleton height={12} widths={["56%"]} />
        <Skeleton height={10} widths={["80%", "34%"]} />
        <Loading size={14}>{es.notes.loadingNote}</Loading>
        <Skeleton height={10} widths={["100%", "92%", "64%"]} />
      </div>
    </div>
  );
}

/** Not drawn in the design: a note the hub could not give, with «Reintentar». */
export function NoteReadFailure({ recordingId, send }: { recordingId: string; send: Send }) {
  const es = useCopy();
  return (
    <div className={styles.centered} role="alert">
      <EmptyState
        action={
          <Button
            onClick={() => void send({ command: "notes_open", recording_id: recordingId })}
            size={36}
            variant="accent"
          >
            {es.notes.retry}
          </Button>
        }
        icon="warning"
        title={es.notes.noteReadFailed}
      >
        {es.notes.noteReadFailedBody}
      </EmptyState>
    </div>
  );
}

/** A meeting that no longer exists (deleted, here or on another Mac). */
export function NoteGone() {
  const es = useCopy();
  return (
    <div className={styles.centered}>
      <EmptyState icon="notes" title={es.notes.noteGone}>
        {es.notes.noteGoneBody}
      </EmptyState>
    </div>
  );
}
