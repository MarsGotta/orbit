import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingAskSnapshot } from "../../bridge/generated/MeetingAskSnapshot";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { moveWithArrows } from "../../design/roving";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Popover } from "../../design/Popover";
import { useCopy } from "../../i18n/CopyProvider";
import { AskAnswer } from "./AskAnswer";
import { questionBytes, voiceStatusText } from "./copy";
import styles from "./AskMenu.module.css";

/** `false` when the daemon refused the order; anything else, it took it. */
type Send = (command: AppCommand) => unknown;

/** «Escucha los últimos 2 / 5 / 10 min»: the minutes app-core offers. */
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
      <span aria-hidden="true">{es.askMenu.windows}</span>
      <div aria-label={es.askMenu.windows} className={styles.choices} role="radiogroup">
        {windows.map(({ minutes: offered }) => (
          <button
            aria-checked={offered === minutes}
            className={styles.window}
            key={offered}
            onClick={() => onChoose(offered)}
            onKeyDown={(event) =>
              moveWithArrows(
                event,
                windows.map((item) => item.minutes),
                minutes,
                onChoose,
              )
            }
            tabIndex={offered === minutes ? 0 : -1}
            role="radio"
            type="button"
          >
            {es.askMenu.window(offered)}
          </button>
        ))}
      </div>
    </div>
  );
}

/** The question, typed, and the minutes of the meeting Deimos listens to. */
function AskForm({
  ask,
  hint,
  onAsk,
  sending,
}: {
  ask: MeetingAskSnapshot;
  hint: string | null;
  onAsk: (question: string, minutes: number) => void;
  /** The question is on its way: it stays here until the daemon takes it. */
  sending: boolean;
}) {
  const es = useCopy();
  const field = useId();
  const [question, setQuestion] = useState("");
  const [minutes, setMinutes] = useState(ask.default_minutes);
  const tooLong = questionBytes(question) > ask.max_question_bytes;
  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (question.trim() !== "" && !tooLong && !sending) onAsk(question.trim(), minutes);
      }}
    >
      <label className={styles.visuallyHidden} htmlFor={field}>
        {es.askMenu.fieldLabel}
      </label>
      <input
        className={styles.field}
        id={field}
        onChange={(event) => setQuestion(event.target.value)}
        placeholder={es.askMenu.placeholder}
        type="text"
        value={question}
      />
      {tooLong && (
        <p className={styles.cause} role="alert">
          {es.meetingQuestion.causes.questionTooLong(
            questionBytes(question),
            ask.max_question_bytes,
          )}
        </p>
      )}
      <WindowChoice minutes={minutes} onChoose={setMinutes} windows={ask.windows} />
      <div className={styles.footer}>
        {hint !== null && <span className={styles.hint}>{hint}</span>}
        <Button disabled={sending} size={32} type="submit" variant="accent">
          {es.askMenu.send}
        </Button>
      </div>
    </form>
  );
}

interface Asked {
  question: string;
  /** The question the daemon showed before this one: its answer is not ours. */
  before: string | null;
}

/**
 * The question asked from the menu. As in the chat, the form gives way to
 * the answer only once the daemon took it; refused, it stays written and the
 * refusal is said where the recording's are (`MeetingOverlays`).
 */
function useAsked(send: Send, current: MeetingQuestionSnapshot | undefined | null) {
  const [asked, setAsked] = useState<Asked | null>(null);
  const [sending, setSending] = useState(false);
  const askNow = (question: string, minutes: number) => {
    const before = current?.id ?? null;
    setSending(true);
    void Promise.resolve(send({ command: "meeting_ask", question, minutes }))
      .then((accepted) => {
        if (accepted !== false) setAsked({ question, before });
      })
      .catch(() => undefined)
      .finally(() => setSending(false));
  };
  return { asked, askNow, clear: () => setAsked(null), sending };
}

function AskSteps({
  ask,
  meeting,
  send,
  onClose,
}: {
  ask: MeetingAskSnapshot;
  meeting: MeetingSnapshot;
  send: Send;
  onClose: () => void;
}) {
  const es = useCopy();
  const current = meeting.question;
  const { asked, askNow, clear, sending } = useAsked(send, current);
  const [kept, setKept] = useState<MeetingQuestionSnapshot | null>(null);
  const live = asked && current && current.id !== asked.before ? current : null;
  // The panel lets an answer go after a while; the menu keeps the last one.
  if (live && live !== kept) setKept(live);
  const voice = meeting.voice_trigger;
  const hint = voice?.state === "listening" ? es.askMenu.voiceHint : voiceStatusText(voice, es);
  return (
    <div className={styles.menu}>
      <h2 className={styles.title}>{es.askMenu.title}</h2>
      {asked === null ? (
        <AskForm ask={ask} hint={hint} onAsk={askNow} sending={sending} />
      ) : (
        <AskAnswer
          asked={asked.question}
          onAgain={() => {
            clear();
            setKept(null);
          }}
          openUrl={(url) => send({ command: "open_url", url })}
          question={live ?? kept}
        />
      )}
      {/* Last, so the focus enters the field first; it sits at the top right. */}
      <button
        aria-label={es.askMenu.close}
        className={styles.close}
        onClick={onClose}
        type="button"
      >
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}

/**
 * «Preguntar a Deimos» on the recording card: a window anchored to it with
 * the question, Deimos thinking and the answer, which also stays in the
 * conversation. Only while app-core offers it. Esc or a press outside closes it.
 */
export function AskMenu({ meeting, send }: { meeting: MeetingSnapshot; send: Send }) {
  const es = useCopy();
  const [open, setOpen] = useState(false);
  const ask = meeting.ask;
  if (!ask) return null;
  return (
    <div className={styles.anchor}>
      <Popover
        label={es.askMenu.title}
        onOpenChange={setOpen}
        open={open}
        trigger={es.recordingCard.ask}
        triggerClassName={styles.trigger}
      >
        <AskSteps ask={ask} meeting={meeting} onClose={() => setOpen(false)} send={send} />
      </Popover>
    </div>
  );
}
