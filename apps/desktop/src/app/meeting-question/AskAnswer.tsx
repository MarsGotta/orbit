import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import { Button } from "../../design/Button";
import { Spinner } from "../../design/Spinner";
import { useCopy } from "../../i18n/CopyProvider";
import { causeText } from "./copy";
import { AnswerText } from "./QuestionCard";
import styles from "./AskMenu.module.css";

function Again({ onAgain, note }: { onAgain: () => void; note?: string }) {
  const es = useCopy();
  return (
    <div className={styles.footer}>
      {note && <span className={styles.hint}>{note}</span>}
      <Button onClick={onAgain} size={28}>
        {es.askMenu.another}
      </Button>
    </div>
  );
}

/**
 * What became of the question: Deimos thinking (also before the daemon
 * says, the instant it is asked), the answer as it arrives, or why there is
 * none. Codes and sizes stay in the log.
 */
export function AskAnswer({
  asked,
  question,
  onAgain,
  openUrl,
}: {
  asked: string;
  question: MeetingQuestionSnapshot | null;
  onAgain: () => void;
  openUrl: (url: string) => void;
}) {
  const es = useCopy();
  const stage = question?.stage ?? "thinking";
  const busy = stage === "thinking" || stage === "responding";
  return (
    <div aria-busy={busy} className={styles.answer}>
      <p className={styles.asked}>{question?.question ?? asked}</p>
      <div aria-live="polite">
        {busy && (
          <p className={styles.status}>
            <Spinner size={12} />
            {stage === "thinking" ? es.meetingQuestion.thinking : es.meetingQuestion.responding}
          </p>
        )}
        {question?.answer && stage !== "failed" && (
          <div className={styles.text}>
            <AnswerText openUrl={openUrl} text={question.answer} />
          </div>
        )}
        {stage === "failed" && question && (
          <>
            <p className={styles.status}>
              {question.sent ? es.meetingQuestion.failed : es.meetingQuestion.notSent}
            </p>
            <p className={styles.cause} role="alert">
              {question.cause ? causeText(question.cause, es) : es.meetingQuestion.causes.tryAgain}
            </p>
          </>
        )}
      </div>
      {stage === "answered" && <Again note={es.askMenu.alsoInConversation} onAgain={onAgain} />}
      {stage === "failed" && <Again onAgain={onAgain} />}
    </div>
  );
}
