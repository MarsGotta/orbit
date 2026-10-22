import type { ReactNode } from "react";
import { Card } from "../../../design/Card";
import { es } from "../../../i18n/es";
import styles from "./Agents.module.css";
import type { Feedback, Place, useAgentConfig } from "./use-agent-config";

export const copy = es.agents;

/** What every card that gives orders needs. */
export interface OrderProps {
  agentId: string;
  busy: boolean;
  feedback: Feedback | null;
  order: ReturnType<typeof useAgentConfig>["order"];
}

export function Panel({ children }: { children: ReactNode }) {
  return (
    <Card>
      <div className={styles.card}>{children}</div>
    </Card>
  );
}

/** The outcome of the last order, where it was given. */
export function FeedbackLine({ feedback, place }: { feedback: Feedback | null; place: Place }) {
  if (feedback?.place !== place) return null;
  if (feedback.kind === "error")
    return (
      <p className={styles.alert} role="alert">
        {feedback.text}
      </p>
    );
  return (
    <p className={styles.hint} role="status">
      {feedback.kind === "saving" ? copy.saving : copy.saved}
    </p>
  );
}

/** What the agent's kind does not let Orbit change, and why. */
export function NotEditable({ editable, cause }: { editable: boolean; cause: string | null }) {
  return editable ? null : <p className={styles.hint}>{copy.notEditable(cause)}</p>;
}

/** Bytes against the hub's ceiling; over it, both sizes. */
export function ByteCount({ bytes, max }: { bytes: number; max: number }) {
  const over = bytes > max;
  return (
    <span className={over ? styles.alert : styles.hint}>
      {over ? copy.overLimit(bytes, max) : copy.bytes(bytes, max)}
    </span>
  );
}
