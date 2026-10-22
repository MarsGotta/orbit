import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import { Button } from "../../design/Button";
import { TextInput } from "../../design/TextInput";
import { es } from "../../i18n/es";
import { useHeldChoice } from "../../state/use-held-choice";
import styles from "./AssistantView.module.css";

type Send = (command: AppCommand) => unknown;

/**
 * An answer given to what the agent asked (`key`: the request and its
 * question): its buttons wait while it travels and until the hub takes the
 * question away, and wake again if it is refused.
 */
function useAnswer(key: string, send: Send) {
  const answer = useHeldChoice(key, send);
  return {
    answered: answer.pending,
    give: (command: AppCommand) => answer.choose(key, command),
  };
}

/** An approval choice in Spanish; one Orbit does not know keeps its own name. */
function choiceLabel(choice: string): string {
  return (es.chat.choices as Record<string, string>)[choice] ?? choice;
}

function Approval({
  pending,
  send,
}: {
  pending: Extract<ChatPending, { kind: "approval" }>;
  send: Send;
}) {
  const { answered, give } = useAnswer(pending.id, send);
  if (pending.choices.length === 0) {
    // The agent named no choices: allow once or refuse, as a 0.9.0 Mac does.
    return (
      <div>
        <Button
          disabled={answered}
          onClick={() => give({ command: "approval", approved: true })}
          variant="accent"
        >
          {es.chat.approve}
        </Button>
        <Button disabled={answered} onClick={() => give({ command: "approval", approved: false })}>
          {es.chat.reject}
        </Button>
      </div>
    );
  }
  return (
    <div className={styles.choices}>
      {pending.choices.map((choice) => (
        <Button
          disabled={answered}
          key={choice}
          onClick={() => give({ command: "approval", approved: choice !== "deny", choice })}
          variant={choice === "once" ? "accent" : undefined}
        >
          {choiceLabel(choice)}
        </Button>
      ))}
    </div>
  );
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
  choices: string[];
  multiple: boolean;
  selected: string[];
  setSelected: (update: (chosen: string[]) => string[]) => void;
  reply: (answer: string) => void;
  answered: boolean;
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
        >
          {choice}
        </Button>
      ))}
    </div>
  );
}

function Clarify({
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
  const { answered, give } = useAnswer(`${pending.id}:${current?.qid ?? ""}`, send);
  useEffect(() => {
    setAnswer("");
    setSelected([]);
  }, [pending.id, current?.qid]);
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
      <p>{current ? current.question : pending.text}</p>
      <Choices
        choices={current ? current.choices : pending.choices}
        multiple={current?.multi_select ?? false}
        answered={answered}
        reply={reply}
        selected={selected}
        setSelected={setSelected}
      />
      <TextInput
        aria-label={es.chat.clarifyLabel}
        onChange={(event) => setAnswer(event.target.value)}
        placeholder={es.assistant.clarifyPlaceholder}
        value={answer}
        variant="dotted"
      />
      <Button
        disabled={answered || (typed === "" && selected.length === 0)}
        // Several chosen travel as a JSON list, which Hermes reads as such.
        onClick={() => reply(typed !== "" ? typed : JSON.stringify(selected))}
        variant="accent"
      >
        {es.chat.clarify}
      </Button>
    </>
  );
}

/**
 * What the agent waits for, with the answers it offers: a button per
 * approval choice; a clarification one question at a time, with its
 * suggestions and free text. No countdown: the hub sends no expiry.
 */
export function PendingCard({ pending, send }: { pending: ChatPending; send: Send }) {
  return (
    <section className={styles.pending}>
      {pending.kind === "approval" ? (
        <>
          <p>{pending.text}</p>
          <Approval pending={pending} send={send} />
        </>
      ) : (
        <Clarify key={pending.id} pending={pending} send={send} />
      )}
    </section>
  );
}
