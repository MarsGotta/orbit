import { useId } from "react";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import { Button } from "../../design/Button";
import { useSpanish } from "../../i18n/AssistantCopy";
import { es } from "../../i18n/es";
import { Clarify, useAnswer, type Send } from "./Clarification";
import styles from "./AssistantView.module.css";
import own from "./PendingCard.module.css";

// The design's order: the main choice first, the wider ones after, refusing last.
const ORDER = ["once", "session", "always"];

/** An approval choice in Spanish; one Orbit does not know keeps its own name. */
function choiceLabel(choice: string): string {
  return (es.chat.choices as Record<string, string>)[choice] ?? choice;
}

/**
 * The agent's choices in the design's order, «deny» apart. A command that
 * deletes keeps only «once»: the hub already offers no other, and the card
 * never shows a wider one.
 */
function ordered(choices: string[], deletes: boolean): { allow: string[]; deny: boolean } {
  // The agent named no choices: allow once or refuse, as a 0.9.0 Mac does.
  if (choices.length === 0) return { allow: [], deny: true };
  const known = ORDER.filter((choice) => choices.includes(choice));
  const other = choices.filter((choice) => choice !== "deny" && !ORDER.includes(choice));
  const allow = [...known, ...other];
  return {
    allow: deletes ? allow.filter((choice) => choice === "once") : allow,
    deny: choices.includes("deny"),
  };
}

function Approval({
  answered,
  pending,
  answer,
}: {
  /** Answered: the buttons wait for the hub to take the request away. */
  answered: boolean;
  pending: Extract<ChatPending, { kind: "approval" }>;
  answer: (approved: boolean, choice?: string) => void;
}) {
  const deletes = pending.deletes !== undefined;
  const { allow, deny } = ordered(pending.choices, deletes);
  const bare = pending.choices.length === 0;
  return (
    <div className={styles.choices}>
      {bare ? (
        <Button disabled={answered} onClick={() => answer(true)} size={32} variant="accent">
          {es.chat.choices.once}
        </Button>
      ) : (
        allow.map((choice) => (
          <Button
            disabled={answered}
            key={choice}
            onClick={() => answer(true, choice)}
            size={32}
            variant={choice === "once" ? "accent" : "outline"}
          >
            {choiceLabel(choice)}
          </Button>
        ))
      )}
      {deletes && <span className={own.oneAtATime}>{es.chatActions.deletesOneAtATime}</span>}
      {deny && (
        <Button
          className={styles.deny}
          disabled={answered}
          onClick={() => (bare ? answer(false) : answer(false, "deny"))}
          size={32}
          variant="ghost"
        >
          {es.chat.choices.deny}
        </Button>
      )}
    </div>
  );
}

/**
 * What the agent waits for, as a card in the thread (and in the floating
 * panel): an approval with what it wants to do and its choices, «Permitir
 * una vez» first and «No permitir» apart (a command that deletes, only
 * those two, with why and what it deletes); a clarification one question at a
 * time, with its suggestions and free text. No countdown: the hub sends no
 * expiry.
 */
export function PendingCard({
  pending,
  send,
  onDecided,
}: {
  pending: ChatPending;
  send: Send;
  /** The choice given to an approval, for the line that remembers it. */
  onDecided?: (choice: string) => void;
}) {
  const named = useSpanish();
  const titleId = useId();
  const approval = pending.kind === "approval";
  // One answer per request: its buttons wait while it travels and until the
  // hub takes the request away, and wake again if it is refused.
  const { answered, give } = useAnswer(pending.id, send);
  const answer = (approved: boolean, choice?: string) => {
    onDecided?.(choice ?? (approved ? "once" : "deny"));
    give(
      choice === undefined
        ? { command: "approval", approved }
        : { command: "approval", approved, choice },
    );
  };
  return (
    <section aria-labelledby={titleId} className={styles.pending}>
      <h3 id={titleId}>
        {approval ? named.assistant.approvalTitle : named.assistant.clarifyTitle}
      </h3>
      {pending.kind === "approval" ? (
        <>
          <p className={styles.approvalText}>{pending.text}</p>
          {pending.deletes !== undefined && (
            <p className={own.deletes}>
              <span aria-hidden="true" className={own.dot} />
              {es.chatActions.deletes(pending.deletes.paths)}
            </p>
          )}
          <Approval answer={answer} answered={answered} pending={pending} />
        </>
      ) : (
        <Clarify key={pending.id} pending={pending} send={send} />
      )}
    </section>
  );
}
