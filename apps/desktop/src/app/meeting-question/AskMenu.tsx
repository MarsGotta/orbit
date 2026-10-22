import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingAskSnapshot } from "../../bridge/generated/MeetingAskSnapshot";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Popover } from "../../design/Popover";
import { useSpanish } from "../../i18n/AssistantCopy";
import { AskAnswer } from "./AskAnswer";
import { questionBytes, voiceStatusText } from "./copy";
import styles from "./AskMenu.module.css";

type Send = (command: AppCommand) => void;

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
  const es = useSpanish();
  return (
    <div aria-label={es.askMenu.windows} className={styles.windows} role="radiogroup">
      <span aria-hidden="true">{es.askMenu.windows}</span>
      {windows.map(({ minutes: offered }) => (
        <button
          aria-checked={offered === minutes}
          className={styles.window}
          key={offered}
          onClick={() => onChoose(offered)}
          role="radio"
          type="button"
        >
          {es.askMenu.window(offered)}
        </button>
      ))}
    </div>
  );
}

/** The question, typed, and the minutes of the meeting Deimos listens to. */
function AskForm({
  ask,
  hint,
  onAsk,
}: {
  ask: MeetingAskSnapshot;
  hint: string | null;
  onAsk: (question: string, minutes: number) => void;
}) {
  const es = useSpanish();
  const field = useId();
  const [question, setQuestion] = useState("");
  const [minutes, setMinutes] = useState(ask.default_minutes);
  const tooLong = questionBytes(question) > ask.max_question_bytes;
  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (question.trim() !== "" && !tooLong) onAsk(question.trim(), minutes);
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
          {es.meetingQuestion.tooLong}
        </p>
      )}
      <WindowChoice minutes={minutes} onChoose={setMinutes} windows={ask.windows} />
      <div className={styles.footer}>
        {hint !== null && <span className={styles.hint}>{hint}</span>}
        <Button size={32} type="submit" variant="accent">
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
  const es = useSpanish();
  const [asked, setAsked] = useState<Asked | null>(null);
  const [kept, setKept] = useState<MeetingQuestionSnapshot | null>(null);
  const current = meeting.question;
  const live = asked && current && current.id !== asked.before ? current : null;
  // The panel lets an answer go after a while; the menu keeps the last one.
  if (live && live !== kept) setKept(live);
  const voice = meeting.voice_trigger;
  const hint = voice?.state === "listening" ? es.askMenu.voiceHint : voiceStatusText(voice, es);
  return (
    <div className={styles.menu}>
      <h2 className={styles.title}>{es.askMenu.title}</h2>
      {asked === null ? (
        <AskForm
          ask={ask}
          hint={hint}
          onAsk={(question, minutes) => {
            send({ command: "meeting_ask", question, minutes });
            setAsked({ question, before: current?.id ?? null });
          }}
        />
      ) : (
        <AskAnswer
          asked={asked.question}
          onAgain={() => {
            setAsked(null);
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
  const es = useSpanish();
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
