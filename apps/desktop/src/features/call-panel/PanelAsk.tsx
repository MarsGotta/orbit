import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingAskSnapshot } from "../../bridge/generated/MeetingAskSnapshot";
import { questionBytes } from "../../app/meeting-question/copy";
import { moveWithArrows } from "../../design/roving";
import { Button } from "../../design/Button";
import { useCopy } from "../../i18n/CopyProvider";
import styles from "./CallPanel.module.css";

/** A pending question keeps the form; refusal is displayed by PanelApp. */
function useQuestionSend(send: (command: AppCommand) => unknown, onDone: () => void) {
  const [sending, setSending] = useState(false);
  const askNow = (question: string, minutes: number) => {
    setSending(true);
    void Promise.resolve()
      .then(() => send({ command: "meeting_ask", question, minutes }))
      .then((accepted) => {
        if (accepted !== false) onDone();
      })
      .catch(() => undefined)
      .finally(() => setSending(false));
  };
  return { sending, askNow };
}

/**
 * «Preguntar sin hablar»: the question, typed, and how many minutes of the
 * meeting go with it (the windows orbit-core offers). Typing is the decision:
 * no confirmation. Esc or «Cancelar» go back.
 */
export function PanelAsk({
  aboutCall,
  ask,
  onDone,
  send,
}: {
  aboutCall: boolean;
  ask: MeetingAskSnapshot;
  onDone: () => void;
  send: (command: AppCommand) => unknown;
}) {
  const es = useCopy();
  const fieldId = useId();
  const [question, setQuestion] = useState("");
  const [minutes, setMinutes] = useState(ask.default_minutes);
  const { sending, askNow } = useQuestionSend(send, onDone);
  const tooLong = questionBytes(question) > ask.max_question_bytes;
  const empty = question.trim() === "";
  return (
    <form
      aria-label={es.meetingQuestion.ask}
      className={styles.card}
      onKeyDown={(event) => {
        if (event.key === "Escape") onDone();
      }}
      onSubmit={(event) => {
        event.preventDefault();
        if (empty || tooLong || sending) return;
        askNow(question.trim(), minutes);
      }}
    >
      <label className={styles.meta} htmlFor={fieldId}>
        {aboutCall ? es.floatingPanel.askAboutCall : es.floatingPanel.askAboutMeeting}
      </label>
      <input
        autoFocus
        className={styles.field}
        id={fieldId}
        onChange={(event) => setQuestion(event.target.value)}
        placeholder={es.meetingQuestion.placeholder}
        type="text"
        value={question}
      />
      <WindowChoice minutes={minutes} onChoose={setMinutes} windows={ask.windows} />
      {tooLong && (
        <p className={styles.refusal} role="alert">
          {es.meetingQuestion.causes.questionTooLong(
            questionBytes(question),
            ask.max_question_bytes,
          )}
        </p>
      )}
      <AskActions disabled={empty || tooLong || sending} onDone={onDone} />
    </form>
  );
}

/** Sending disables repeat submission while Cancel remains an explicit dismissal. */
function AskActions({ disabled, onDone }: { disabled: boolean; onDone: () => void }) {
  const es = useCopy();
  return (
    <div className={`${styles.actions} ${styles.end}`}>
      <Button onClick={onDone} size={32} variant="ghost">
        {es.meetingQuestion.cancel}
      </Button>
      <Button disabled={disabled} size={32} type="submit" variant="accent">
        {es.meetingQuestion.send}
      </Button>
    </div>
  );
}

/** «Escucha los últimos 2 / 5 / 10 min»: the minutes of the meeting that go with it. */
function WindowChoice({
  minutes,
  onChoose,
  windows,
}: {
  minutes: number;
  onChoose: (minutes: number) => void;
  windows: MeetingAskSnapshot["windows"];
}) {
  const es = useCopy();
  return (
    <div className={styles.windows}>
      <span aria-hidden="true">{es.floatingPanel.listenLast}</span>
      <div aria-label={es.floatingPanel.listenLast} className={styles.choices} role="radiogroup">
        {windows.map((window) => (
          <button
            aria-checked={window.minutes === minutes}
            className={styles.window}
            key={window.minutes}
            onClick={() => onChoose(window.minutes)}
            onKeyDown={(event) =>
              moveWithArrows(
                event,
                windows.map((item) => item.minutes),
                minutes,
                onChoose,
              )
            }
            tabIndex={window.minutes === minutes ? 0 : -1}
            role="radio"
            type="button"
          >
            {es.floatingPanel.minutes(window.minutes)}
          </button>
        ))}
      </div>
    </div>
  );
}
