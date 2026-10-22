import type { AppCommand } from "../../bridge/generated";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import { answerPieces, causeText, linkText } from "../../app/meeting-question/copy";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Loading } from "../../design/Spinner";
import { TextLink } from "../../design/TextLink";
import { useSpanish } from "../../i18n/AssistantCopy";
import { useNow } from "../../use-now";
import { PendingCard } from "../assistant/PendingCard";
import { MarkdownView } from "../notes/MarkdownView";
import { Remaining } from "./CallPanel";
import styles from "./CallPanel.module.css";

type Copy = ReturnType<typeof useSpanish>;

/**
 * The question to Deimos and what follows it: Deimos thinking, what Deimos
 * asks to do meanwhile, the answer as it arrives (with the bar of the 45 s
 * the daemon shows it), or why there is none. The answer also stays in the
 * conversation; «Otra pregunta» while asking is offered.
 */
export function PanelAnswer({
  onAsk,
  openUrl,
  pending,
  question,
  send,
}: {
  onAsk?: () => void;
  openUrl: (url: string) => void;
  pending: ChatPending | null | undefined;
  question: MeetingQuestionSnapshot;
  send: (command: AppCommand) => void;
}) {
  const es = useSpanish();
  const inFlight = question.stage === "thinking" || question.stage === "responding";
  const asked = inFlight && pending ? pending : null;
  const finished = question.stage === "answered" || question.stage === "failed";
  return (
    <section
      aria-busy={inFlight}
      aria-label={es.meetingQuestion.panelLabel}
      className={styles.card}
    >
      <div className={styles.head}>
        <p className={styles.asked}>{question.question}</p>
        <button
          aria-label={es.floatingPanel.close}
          className={styles.close}
          onClick={() => send({ command: "meeting_question_close", id: question.id })}
          type="button"
        >
          <Icon name="close" size={14} />
        </button>
      </div>
      {asked?.kind === "approval" ? (
        <Approval pending={asked} send={send} />
      ) : (
        <div aria-live="polite" className={styles.body}>
          {body(question, openUrl, es)}
          {asked && <PendingCard pending={asked} send={send} />}
        </div>
      )}
      {finished && (
        <div className={styles.foot}>
          <span className={styles.meta}>
            {question.stage === "answered" ? es.floatingPanel.alsoInConversation : null}
          </span>
          {onAsk && (
            <Button onClick={onAsk} size={28}>
              {es.floatingPanel.another}
            </Button>
          )}
        </div>
      )}
      {question.stage === "answered" && question.shown_until_ms != null && (
        <AnswerShown untilMs={question.shown_until_ms} />
      )}
    </section>
  );
}

/** The bar along the bottom edge that empties until the daemon lets the answer go. */
function AnswerShown({ untilMs }: { untilMs: number }) {
  const now = useNow(1_000);
  return <Remaining edge nowMs={now} untilMs={untilMs} />;
}

function body(question: MeetingQuestionSnapshot, openUrl: (url: string) => void, es: Copy) {
  switch (question.stage) {
    case "thinking":
      return <Loading size={12}>{es.meetingQuestion.thinking}</Loading>;
    case "responding":
      return (
        <>
          <Loading size={12}>{es.meetingQuestion.responding}</Loading>
          <AnswerText openUrl={openUrl} text={question.answer ?? ""} />
        </>
      );
    case "answered":
      return <AnswerText openUrl={openUrl} text={question.answer ?? ""} />;
    case "failed":
      return (
        <>
          <p className={styles.title}>
            {question.sent ? es.meetingQuestion.failed : es.meetingQuestion.notSent}
          </p>
          <p className={styles.cause} role="alert">
            {question.cause ? causeText(question.cause, es) : es.meetingQuestion.causes.tryAgain}
          </p>
        </>
      );
  }
}

/**
 * The answer, in the Markdown Deimos writes, with its web links (bare ones
 * too) opening outside Orbit, each with its host beside it.
 */
function AnswerText({ openUrl, text }: { openUrl: (url: string) => void; text: string }) {
  // Web links become Markdown links; any other bracket stays literal, so a
  // link to anything but the web is shown as written, never as a link.
  const source = answerPieces(text)
    .map((piece) =>
      "url" in piece ? `[${piece.text}](${piece.url})` : piece.text.replaceAll("[", "\\["),
    )
    .join("");
  return (
    <div className={styles.answer}>
      <MarkdownView
        link={(url, label) => (
          <TextLink
            href={url}
            onClick={(event) => {
              event.preventDefault();
              openUrl(url);
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

/** What Deimos asks to do while it answers, with the choices it offers. */
function Approval({
  pending,
  send,
}: {
  pending: Extract<ChatPending, { kind: "approval" }>;
  send: (command: AppCommand) => void;
}) {
  const es = useSpanish();
  const labels: Record<string, string> = es.floatingPanel.choices;
  // An agent that names no choices offers allowing once or refusing, sent
  // without a choice as a 0.9.0 Mac does.
  const named = pending.choices.length > 0;
  const choices = named ? pending.choices : ["once", "deny"];
  return (
    <>
      <p className={styles.permission}>{pending.text}</p>
      <div className={styles.actions}>
        {choices.map((choice) => {
          const approved = choice !== "deny";
          return (
            <Button
              className={choice === "deny" ? styles.deny : undefined}
              key={choice}
              onClick={() =>
                send(
                  named
                    ? { command: "approval", approved, choice }
                    : { command: "approval", approved },
                )
              }
              size={28}
              variant={choice === "once" ? "accent" : choice === "deny" ? "ghost" : "outline"}
            >
              {labels[choice] ?? choice}
            </Button>
          );
        })}
      </div>
    </>
  );
}
