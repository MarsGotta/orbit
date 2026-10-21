import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import { Button } from "../../design/Button";
import { TextInput } from "../../design/TextInput";
import { es } from "../../i18n/es";
import styles from "./AssistantView.module.css";

export function PendingCard({
  pending,
  send,
}: {
  pending: ChatPending;
  send: (command: AppCommand) => void;
}) {
  const [answer, setAnswer] = useState("");
  useEffect(() => setAnswer(""), [pending.id]);

  if (pending.kind === "approval") {
    return (
      <section className={styles.pending}>
        <p>{pending.text}</p>
        <div>
          <Button onClick={() => send({ command: "approval", approved: true })} variant="accent">
            {es.chat.approve}
          </Button>
          <Button onClick={() => send({ command: "approval", approved: false })}>
            {es.chat.reject}
          </Button>
        </div>
      </section>
    );
  }
  return (
    <section className={styles.pending}>
      <p>{pending.text}</p>
      <TextInput
        aria-label={es.chat.clarifyLabel}
        onChange={(event) => setAnswer(event.target.value)}
        placeholder={es.assistant.clarifyPlaceholder}
        value={answer}
        variant="dotted"
      />
      <Button
        disabled={!answer.trim()}
        onClick={() => send({ command: "clarify", answer: answer.trim() })}
        variant="accent"
      >
        {es.chat.clarify}
      </Button>
    </section>
  );
}
