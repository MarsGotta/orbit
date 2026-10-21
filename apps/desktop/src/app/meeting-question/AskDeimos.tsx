import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingAskSnapshot } from "../../bridge/generated/MeetingAskSnapshot";
import type { VoiceStatusSnapshot } from "../../bridge/generated/VoiceStatusSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { useSpanish } from "../../i18n/AssistantCopy";
import { questionBytes, voiceStatusText } from "./copy";
import styles from "./MeetingQuestion.module.css";

export interface AskDeimosProps {
  ask: MeetingAskSnapshot;
  voiceStatus?: VoiceStatusSnapshot;
  send: (command: AppCommand) => void;
  /** The field closed: the question went, or Mars cancelled. */
  onDone: () => void;
}

/**
 * P9-12: «Preguntar a Deimos» during a recording — the question, typed,
 * and how many minutes of the meeting go with it (the windows app-core
 * offers, 2 by default). Typing is the decision: no confirmation.
 */
export function AskDeimos({ ask, voiceStatus, send, onDone }: AskDeimosProps) {
  const es = useSpanish();
  const [question, setQuestion] = useState("");
  const [minutes, setMinutes] = useState(ask.default_minutes);
  const bytes = questionBytes(question);
  const tooLong = bytes > ask.max_question_bytes;
  const empty = question.trim() === "";
  const voice = voiceStatusText(voiceStatus, es);
  return (
    <form
      aria-label={es.meetingQuestion.ask}
      className={styles.ask}
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
      <textarea
        aria-label={es.meetingQuestion.fieldLabel}
        autoFocus
        className={styles.field}
        onChange={(event) => setQuestion(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault();
            event.currentTarget.form?.requestSubmit();
          }
        }}
        placeholder={es.meetingQuestion.placeholder}
        rows={2}
        value={question}
      />
      <WindowChoice
        minutes={minutes}
        onChoose={setMinutes}
        windows={ask.windows.map((window) => window.minutes)}
      />
      {tooLong && (
        <p className={styles.cause} role="alert">
          {es.meetingQuestion.tooLong}
        </p>
      )}
      {voice !== null && <p className={styles.hint}>{voice}</p>}
      <div className={styles.actions}>
        <Button onClick={onDone} size={26} variant="ghost">
          {es.meetingQuestion.cancel}
        </Button>
        <Button disabled={empty || tooLong} size={26} type="submit" variant="accent">
          {es.meetingQuestion.send}
        </Button>
      </div>
    </form>
  );
}

/** The minutes of the meeting that go with the question. */
function WindowChoice({
  minutes,
  onChoose,
  windows,
}: {
  minutes: number;
  onChoose: (minutes: number) => void;
  windows: number[];
}) {
  return (
    <div aria-label={es.meetingQuestion.windowLabel} className={styles.windows} role="radiogroup">
      {windows.map((window) => (
        <button
          aria-checked={window === minutes}
          className={styles.window}
          key={window}
          onClick={() => onChoose(window)}
          role="radio"
          type="button"
        >
          {es.meetingQuestion.window(window)}
        </button>
      ))}
    </div>
  );
}
