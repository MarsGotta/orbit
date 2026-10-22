import { es } from "../../../i18n/es";
import styles from "./Agents.module.css";
import type { Feedback, Place, useAgentConfig } from "./use-agent-config";

export const copy = es.agents;

/** What every part that gives orders needs. */
export interface OrderProps {
  agentId: string;
  busy: boolean;
  feedback: Feedback | null;
  order: ReturnType<typeof useAgentConfig>["order"];
}

/**
 * The outcome of the last order, where it was given. `errorsOnly` where the
 * button itself says «Guardando…» and «✓ Guardado».
 */
export function FeedbackLine({
  feedback,
  place,
  errorsOnly,
}: {
  feedback: Feedback | null;
  place: Place;
  errorsOnly?: boolean;
}) {
  if (feedback?.place !== place) return null;
  if (feedback.kind === "error")
    return (
      <p className={styles.alert} role="alert">
        {feedback.text}
      </p>
    );
  if (errorsOnly === true) return null;
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
    <span aria-live="polite" className={over ? styles.alert : styles.count}>
      {over ? copy.overLimit(bytes, max) : copy.bytes(bytes, max)}
    </span>
  );
}
