import { useCopy } from "../../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { Dialog } from "../../design/Dialog";
import { Field } from "../../design/Field";

import { AudioSourceField } from "./AudioSourceField";
import { StartBlocker } from "./StartBlocker";
import { clock, startBlocker, titleSuggestion } from "./start-options";
import { useAudioChoice, useStart } from "./start-state";
import styles from "./MeetingStartDialog.module.css";

export interface MeetingStartDialogProps {
  onClose: () => void;
  send: (command: AppCommand) => void;
  snapshot: AppSnapshot;
  /** The latest refusal of a recording order, while it is on screen. */
  error: string | null;
  /** Clears that refusal: pressing again starts afresh. */
  onErrorDismiss: () => void;
}

/**
 * «Nueva grabación»: the title (with the event about to start as a
 * suggestion), what audio to record and the limit, said before it starts.
 * «Empezar a grabar» says «Empezando…» until the recording begins, then the
 * window closes; a refusal is said inside it.
 */
export function MeetingStartDialog(props: MeetingStartDialogProps) {
  const es = useCopy();
  const { onClose, send, snapshot, error, onErrorDismiss } = props;
  const [now] = useState(() => Date.now());
  const [title, setTitle] = useState("");
  const choice = useAudioChoice(snapshot, send);
  const start = useStart(snapshot, { error, onErrorDismiss }, onClose);
  const blockerId = useId();
  const statusId = useId();
  const { system_timezone: zone } = snapshot;
  const blocker = startBlocker(snapshot, choice.field.source);
  const { audio } = choice;
  const footer = (
    <StartFooter
      describedBy={blocker ? blockerId : audio === null ? statusId : undefined}
      disabled={blocker !== null || audio === null}
      onCancel={onClose}
      onStart={
        audio === null
          ? null
          : () => {
              start.press();
              send({ command: "meeting_start", title: title.trim(), audio });
            }
      }
      pending={start.pending}
    />
  );
  return (
    <Dialog
      footer={footer}
      onClose={onClose}
      subtitle={es.meeting.startSubtitle}
      title={es.meeting.startDialog}
    >
      <div className={styles.body}>
        {blocker && (
          <div id={blockerId}>
            <StartBlocker blocker={blocker} send={send} />
          </div>
        )}
        <TitleField
          onTitle={setTitle}
          placeholder={es.meeting.untitled(clock(now, zone))}
          suggestion={titleSuggestion(snapshot, now)}
          title={title}
        />
        <AudioSourceField {...choice.field} statusId={statusId} />
        <p className={styles.meta}>{es.meeting.limit}</p>
        {start.refusal !== null && (
          <p className={styles.refusal} role="alert">
            {start.refusal}
          </p>
        )}
      </div>
    </Dialog>
  );
}

/** «Título», with the event's title to use while it is empty. */
function TitleField(props: {
  title: string;
  onTitle: (title: string) => void;
  placeholder: string;
  suggestion: { title: string; label: string } | null;
}) {
  const es = useCopy();
  const { title, onTitle, placeholder, suggestion } = props;
  return (
    <div className={styles.title}>
      <Field
        label={es.meeting.titleLabel}
        onChange={(event) => onTitle(event.currentTarget.value)}
        placeholder={placeholder}
        size={40}
        value={title}
      />
      {title === "" && suggestion ? (
        <button
          className={styles.suggestion}
          onClick={() => onTitle(suggestion.title)}
          type="button"
        >
          {suggestion.label}
        </button>
      ) : (
        <p className={styles.meta}>{es.meeting.titleHint}</p>
      )}
    </div>
  );
}

/**
 * «Cancelar» and «Empezar a grabar»: what keeps it disabled is said once,
 * in the window (the blocker's row, or why no application can be chosen),
 * and the button points to it.
 */
function StartFooter(props: {
  describedBy: string | undefined;
  disabled: boolean;
  onCancel: () => void;
  onStart: (() => void) | null;
  pending: boolean;
}) {
  const es = useCopy();
  const { describedBy, disabled, onCancel, onStart, pending } = props;
  return (
    <>
      <Button onClick={onCancel} size={36} variant="ghost">
        {es.meeting.cancel}
      </Button>
      <Button
        aria-describedby={describedBy}
        disabled={disabled}
        doneLabel={es.meeting.startPending}
        feedback={pending ? "pending" : "idle"}
        onClick={() => onStart?.()}
        pendingLabel={es.meeting.startPending}
        size={36}
        variant="danger"
      >
        <span aria-hidden="true" className={styles.recordDot} />
        {es.meeting.record}
      </Button>
    </>
  );
}
