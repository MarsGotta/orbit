import { useCopy } from "../i18n/CopyProvider";
import { useState } from "react";
import type { AppCommand } from "../bridge/generated";
import type { MeetingSnapshot } from "../bridge/generated/MeetingSnapshot";
import { Button } from "../design/Button";
import { useFeedback } from "../design/feedback";

import styles from "./DiscardRecording.module.css";

type Send = (command: AppCommand) => unknown;

/**
 * «Terminar sin guardar», asked in place: the first press only asks, and
 * the question stays with the recording it asked about. Offered while it
 * records or is paused, never while it starts or is being saved.
 */
export function useDiscardQuestion(meeting: MeetingSnapshot) {
  const [askedFor, setAskedFor] = useState<string | null>(null);
  const recordingId = meeting.recording_id;
  const offered =
    recordingId !== null && (meeting.phase === "recording" || meeting.phase === "paused");
  return {
    offered,
    asking: offered && askedFor === recordingId,
    ask: () => setAskedFor(recordingId),
    keep: () => setAskedFor(null),
  };
}

/** The quiet button under the card's own controls. */
export function DiscardButton({ onAsk }: { onAsk: () => void }) {
  const es = useCopy();
  return (
    <Button className={styles.ask} onClick={onAsk} size={28} variant="ghost">
      {es.discard.action}
    </Button>
  );
}

/**
 * The question, in place of the card's controls: what it deletes, and
 * «Borrar y terminar» or «Seguir grabando».
 */
export function DiscardQuestion({
  onKeep,
  recordingId,
  send,
}: {
  onKeep: () => void;
  recordingId: string;
  send: Send;
}) {
  const es = useCopy();
  const confirm = useFeedback();
  return (
    <div aria-label={es.discard.question} className={styles.question} role="group">
      <p className={styles.text}>
        <strong>{es.discard.question}</strong> {es.discard.detail}
      </p>
      <div className={styles.answers}>
        <Button
          feedback={confirm.feedback}
          onClick={() =>
            confirm.track(
              Promise.resolve(send({ command: "meeting_discard", recording_id: recordingId })),
            )
          }
          pendingLabel={es.discard.pending}
          doneLabel={es.discard.pending}
          size={32}
          variant="dangerSoft"
        >
          {es.discard.confirm}
        </Button>
        <Button onClick={onKeep} size={32}>
          {es.discard.keep}
        </Button>
      </div>
    </div>
  );
}
