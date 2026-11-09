import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { BriefingPreviousActions } from "../../bridge/generated/BriefingPreviousActions";
import { Checkbox } from "../../design/Checkbox";
import { Loading } from "../../design/Spinner";

import type { Send } from "./send";
import styles from "./BriefingView.module.css";

/** Why the earlier note's tasks are not known, in the words of its state. */
function notWritten(noteState: string | null): string {
  if (noteState === "failed") return getCopy().briefing.pendingNoteFailed;
  if (noteState !== null && ["waiting_transcript", "queued", "generating"].includes(noteState))
    return getCopy().briefing.pendingWriting;
  return getCopy().briefing.pendingNoNote;
}

/**
 * «Pendiente de la última vez»: the tasks of the series' earlier note, marked
 * as its Actions tab marks them. A series still being read says so; one that
 * could not be read says why; neither is «nothing pending».
 */
export function PreviousActions({
  actions,
  send,
}: {
  actions: BriefingPreviousActions;
  send: Send;
}) {
  const es = useCopy();
  return (
    <div className={styles.pending}>
      <h3>{es.briefing.pending}</h3>
      <PendingBody actions={actions} send={send} />
    </div>
  );
}

function PendingBody({ actions, send }: { actions: BriefingPreviousActions; send: Send }) {
  const es = useCopy();
  switch (actions.state) {
    case "unknown":
      return actions.cause === "series_not_read" ? (
        <Loading size={12}>{es.briefing.pendingReading}</Loading>
      ) : (
        <p className={styles.quiet}>
          {es.briefing.pendingFailed}
          <small>{es.briefingCause(actions.cause)}</small>
        </p>
      );
    case "no_previous_meeting":
      return <p className={styles.quiet}>{es.briefing.pendingFirst}</p>;
    case "note_not_written":
      return <p className={styles.quiet}>{notWritten(actions.note_state)}</p>;
    case "known":
      if (actions.items.length === 0)
        return <p className={styles.quiet}>{es.briefing.pendingNone}</p>;
      return (
        <ul className={styles.tasks}>
          {actions.items.map((item) => (
            <li key={item.id}>
              <Checkbox
                checked={item.done}
                onCheckedChange={(done) =>
                  send({
                    command: "note_action_toggle",
                    recording_id: actions.recording_id,
                    action_id: item.id,
                    done,
                  })
                }
              >
                {es.briefing.pendingTask(item.owner, item.text)}
              </Checkbox>
            </li>
          ))}
        </ul>
      );
  }
}
