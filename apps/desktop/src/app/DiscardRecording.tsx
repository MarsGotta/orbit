import { useCopy } from "../i18n/CopyProvider";
import { useId, useLayoutEffect, useRef, useState, type Ref } from "react";
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
  const asking = offered && askedFor === recordingId;
  const trigger = useRef<HTMLButtonElement>(null);
  const restoreFocus = useRef(false);
  useLayoutEffect(() => {
    if (!asking && restoreFocus.current) {
      restoreFocus.current = false;
      trigger.current?.focus();
    }
  }, [asking]);
  return {
    offered,
    asking,
    trigger,
    ask: () => setAskedFor(recordingId),
    keep: () => {
      restoreFocus.current = true;
      setAskedFor(null);
    },
  };
}

/** The quiet button under the card's own controls. */
export function DiscardButton({ onAsk, ref }: { onAsk: () => void; ref?: Ref<HTMLButtonElement> }) {
  const es = useCopy();
  return (
    <Button className={styles.ask} onClick={onAsk} ref={ref} size={28} variant="ghost">
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
  const detailId = useId();
  const keep = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => keep.current?.focus(), []);
  return (
    <div
      aria-label={es.discard.question}
      aria-describedby={detailId}
      className={styles.question}
      role="group"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onKeep();
        }
      }}
    >
      <p className={styles.text}>
        <strong>{es.discard.question}</strong> <span id={detailId}>{es.discard.detail}</span>
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
        <Button onClick={onKeep} ref={keep} size={32}>
          {es.discard.keep}
        </Button>
      </div>
    </div>
  );
}
