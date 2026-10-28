import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingAskSnapshot } from "../../bridge/generated/MeetingAskSnapshot";
import { questionBytes } from "../../app/meeting-question/copy";
import { Button } from "../../design/Button";
import { useCopy } from "../../i18n/CopyProvider";
import styles from "./CallPanel.module.css";

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
  send: (command: AppCommand) => void;
}) {
  const es = useCopy();
  const fieldId = useId();
  const [question, setQuestion] = useState("");
  const [minutes, setMinutes] = useState(ask.default_minutes);
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
        if (empty || tooLong) return;
        send({ command: "meeting_ask", question: question.trim(), minutes });
        onDone();
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
          {es.meetingQuestion.tooLong}
        </p>
      )}
      <div className={`${styles.actions} ${styles.end}`}>
        <Button onClick={onDone} size={32} variant="ghost">
          {es.meetingQuestion.cancel}
        </Button>
        <Button disabled={empty || tooLong} size={32} type="submit" variant="accent">
          {es.meetingQuestion.send}
        </Button>
      </div>
    </form>
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
    <div aria-label={es.floatingPanel.listenLast} className={styles.windows} role="radiogroup">
      <span aria-hidden="true">{es.floatingPanel.listenLast}</span>
      {windows.map((window) => (
        <button
          aria-checked={window.minutes === minutes}
          className={styles.window}
          key={window.minutes}
          onClick={() => onChoose(window.minutes)}
          role="radio"
          type="button"
        >
          {es.floatingPanel.minutes(window.minutes)}
        </button>
      ))}
    </div>
  );
}
