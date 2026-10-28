import { getCopy } from "../../i18n";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import { Button } from "../../design/Button";
import { TextLink } from "../../design/TextLink";
import { MarkdownView } from "../../features/notes/MarkdownView";

import { useCopy } from "../../i18n/CopyProvider";
import { answerPieces, causeText, linkText } from "./copy";
import styles from "./MeetingQuestion.module.css";

export interface QuestionCardProps {
  question: MeetingQuestionSnapshot;
  send: (command: AppCommand) => void;
  /** Opens a web link of the answer (the panel's own order). */
  openUrl?: (url: string) => void;
}

/**
 * The question to Deimos in the floating panel: Deimos thinking,
 * the answer as it arrives, or why there is none. It stays
 * until the daemon lets it go (45 s after the answer, 8 s after a failure)
 * or the user closes it; the answer also stays in the conversation.
 */
export function QuestionCard({ question, send, openUrl }: QuestionCardProps) {
  const es = useCopy();
  const failed = question.stage === "failed";
  return (
    <section
      aria-busy={question.stage === "thinking" || question.stage === "responding"}
      aria-label={es.meetingQuestion.panelLabel}
      className={`${styles.card} ${failed ? styles.failed : ""}`}
    >
      <p className={styles.question}>{es.meetingQuestion.asked(question.question)}</p>
      <div aria-live="polite" className={styles.answer}>
        {body(question, openUrl, es)}
      </div>
      <div className={styles.actions}>
        {question.stage === "answered" && (
          <span className={styles.hint}>{es.meetingQuestion.alsoInConversation}</span>
        )}
        <Button
          onClick={() => send({ command: "meeting_question_close", id: question.id })}
          size={28}
          variant="ghost"
        >
          {es.meetingQuestion.close}
        </Button>
      </div>
    </section>
  );
}

function body(
  question: MeetingQuestionSnapshot,
  openUrl: QuestionCardProps["openUrl"],
  copy = getCopy(),
) {
  switch (question.stage) {
    case "thinking":
      return <p className={styles.status}>{copy.meetingQuestion.thinking}</p>;
    case "responding":
      return (
        <>
          <p className={styles.status}>{copy.meetingQuestion.responding}</p>
          <AnswerText openUrl={openUrl} text={question.answer ?? ""} />
        </>
      );
    case "answered":
      return <AnswerText openUrl={openUrl} text={question.answer ?? ""} />;
    case "failed":
      return (
        <>
          <p className={styles.status}>
            {question.sent ? copy.meetingQuestion.failed : copy.meetingQuestion.notSent}
          </p>
          <p className={styles.cause} role="alert">
            {question.cause
              ? causeText(question.cause, copy)
              : copy.meetingQuestion.causes.tryAgain}
          </p>
        </>
      );
  }
}

/**
 * The answer, in the Markdown Deimos writes, with its web links,
 * bare ones too, as links that open outside Orbit through the panel.
 */
export function AnswerText({
  openUrl,
  text,
}: {
  openUrl: QuestionCardProps["openUrl"];
  text: string;
}) {
  // Its web links become Markdown links; any other bracket stays literal, so
  // a link to anything but the web is shown as written, never as a link.
  const source = answerPieces(text)
    .map((piece) =>
      "url" in piece ? `[${piece.text}](${piece.url})` : piece.text.replaceAll("[", "\\["),
    )
    .join("");
  return (
    <div className={styles.text}>
      <MarkdownView
        link={(url, label) => (
          <TextLink
            href={url}
            onClick={(event) => {
              event.preventDefault();
              openUrl?.(url);
            }}
            rel="noreferrer"
          >
            {linkText({ text: label, url })}
          </TextLink>
        )}
        source={source}
      />
    </div>
  );
}
