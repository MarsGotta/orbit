import type { ReactNode } from "react";
import { es } from "../i18n/es";
import { Button } from "./Button";
import { Spinner } from "./Spinner";
import styles from "./ProgressCard.module.css";

export interface ProgressCardProps {
  /** What is being done: «Rehaciendo la transcripción…». */
  children: ReactNode;
  /** How long is left, as the daemon estimates it: «unos 3 min». */
  remaining?: string;
  /** How far it has gone, 0 to 100; without it (in the queue) there is no bar. */
  percent?: number;
  /** It ended without doing it: the card turns to a warning that stays until closed. */
  failed?: boolean;
  onClose?: () => void;
}

/** A long piece of work the person started and can leave running, as it goes. */
export function ProgressCard({ children, remaining, percent, failed, onClose }: ProgressCardProps) {
  const shown = percent === undefined ? null : Math.max(0, Math.min(100, percent));
  return (
    <div className={styles.card} data-tone={failed ? "warn" : "work"} role="status">
      <div className={styles.row}>
        {failed ? <span aria-hidden="true" className={styles.dot} /> : <Spinner accent size={13} />}
        <span className={styles.text}>{children}</span>
        {!failed && remaining && <span className={styles.remaining}>{remaining}</span>}
        {failed && onClose && (
          <Button onClick={onClose} size={28} variant="ghost">
            {es.controls.close}
          </Button>
        )}
      </div>
      {!failed && shown !== null && (
        <div aria-hidden="true" className={styles.track}>
          <span className={styles.bar} data-progress="" style={{ width: `${shown}%` }} />
        </div>
      )}
    </div>
  );
}

/** What is being redone: still in sight, dimmed, and out of reach until it ends. */
export function BeingRedone({ active, children }: { active: boolean; children: ReactNode }) {
  return (
    <div
      aria-busy={active || undefined}
      className={styles.redone}
      data-dimmed={active}
      inert={active}
    >
      {children}
    </div>
  );
}
