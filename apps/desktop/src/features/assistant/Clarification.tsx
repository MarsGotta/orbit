import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { useHeldChoice } from "../../state/use-held-choice";
import styles from "./AssistantView.module.css";

export type Send = (command: AppCommand) => unknown;

/**
 * An answer given to what the agent asked (`key`: the request and its
 * question): its buttons wait while it travels and until the hub takes the
 * question away, and wake again if it is refused.
 */
export function useAnswer(key: string, send: Send) {
  const answer = useHeldChoice(key, send);
  return {
    answered: answer.pending,
    give: (command: AppCommand) => answer.choose(key, command),
  };
}

/** How many questions the request had when it was first shown, for «Pregunta 1 de 2». */
function useAsked(id: string, remaining: number): number {
  const [asked, setAsked] = useState({ id, count: remaining });
  useEffect(() => {
    setAsked((known) =>
      known.id === id && known.count >= remaining ? known : { id, count: remaining },
    );
  }, [id, remaining]);
  return asked.id === id ? Math.max(asked.count, remaining) : remaining;
}

function Choices({
  choices,
  multiple,
  selected,
  setSelected,
  reply,
  answered,
}: {
  answered: boolean;
  choices: string[];
  multiple: boolean;
  selected: string[];
  setSelected: (update: (chosen: string[]) => string[]) => void;
  reply: (answer: string) => void;
}) {
  if (choices.length === 0) return null;
  const toggle = (choice: string) =>
    setSelected((chosen) =>
      chosen.includes(choice) ? chosen.filter((known) => known !== choice) : [...chosen, choice],
    );
  return (
    <div className={styles.choices}>
      {choices.map((choice) => (
        <Button
          aria-pressed={multiple ? selected.includes(choice) : undefined}
          disabled={answered}
          key={choice}
          onClick={() => (multiple ? toggle(choice) : reply(choice))}
          size={32}
        >
          {choice}
        </Button>
      ))}
    </div>
  );
}

/** The free answer and «Responder», which also sends the suggestions chosen. */
function ReplyForm({
  answer,
  onAnswer,
  onReply,
  ready,
}: {
  answer: string;
  onAnswer: (text: string) => void;
  onReply: () => void;
  ready: boolean;
}) {
  return (
    <form
      className={styles.reply}
      onSubmit={(event) => {
        event.preventDefault();
        if (ready) onReply();
      }}
    >
      <input
        aria-label={es.chat.clarifyLabel}
        onChange={(event) => onAnswer(event.target.value)}
        placeholder={es.assistant.clarifyPlaceholder}
        value={answer}
      />
      <Button disabled={!ready} size={32} type="submit" variant="accent">
        {es.chat.clarify}
      </Button>
    </form>
  );
}

export function Clarify({
  pending,
  send,
}: {
  pending: Extract<ChatPending, { kind: "clarify" }>;
  send: Send;
}) {
  const [answer, setAnswer] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  // One question at a time: the hub removes each once answered.
  const current = pending.questions[0] ?? null;
  const asked = useAsked(pending.id, pending.questions.length);
  useEffect(() => {
    setAnswer("");
    setSelected([]);
  }, [pending.id, current?.qid]);
  const { answered, give } = useAnswer(`${pending.id}:${current?.qid ?? ""}`, send);
  const reply = (text: string) =>
    give(
      current
        ? { command: "clarify", answer: text, question_id: current.qid }
        : { command: "clarify", answer: text },
    );
  const typed = answer.trim();
  return (
    <>
      {asked > 1 && (
        <span className={styles.questionCount}>
          {es.chat.questionOf(asked - pending.questions.length + 1, asked)}
        </span>
      )}
      <p className={styles.pendingText}>{current ? current.question : pending.text}</p>
      <Choices
        choices={current ? current.choices : pending.choices}
        answered={answered}
        multiple={current?.multi_select ?? false}
        reply={reply}
        selected={selected}
        setSelected={setSelected}
      />
      <ReplyForm
        answer={answer}
        onAnswer={setAnswer}
        // Several chosen travel as a JSON list, which Hermes reads as such.
        onReply={() => reply(typed !== "" ? typed : JSON.stringify(selected))}
        ready={!answered && (typed !== "" || selected.length > 0)}
      />
    </>
  );
}
