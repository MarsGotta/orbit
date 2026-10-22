import type { ReactNode } from "react";
import { es } from "../i18n/es";
import { Button } from "./Button";
import styles from "./ViewNotice.module.css";

export interface ViewNoticeProps {
  /** What is wrong, or what it waits for: «Inicia sesión otra vez en Claude». */
  title: string;
  /** Why, and what stays as it was. */
  children: ReactNode;
  /** The button that fixes it: «Iniciar sesión en Claude». */
  actionLabel: string;
  onAction: () => void;
  /** While the fix is under way: the button turns to «Esperando…» and cannot be pressed again. */
  waiting?: boolean;
}

/** A notice inside a view, with the button that fixes it on its right. */
export function ViewNotice({ title, children, actionLabel, onAction, waiting }: ViewNoticeProps) {
  return (
    <div className={styles.notice} role="status">
      <span aria-hidden="true" className={styles.dot} />
      <span className={styles.text}>
        <strong className={styles.title}>{title}</strong>
        <span className={styles.body}>{children}</span>
      </span>
      <Button
        feedback={waiting ? "pending" : "idle"}
        onClick={onAction}
        pendingLabel={es.controls.waiting}
        size={32}
        variant="accent"
      >
        {actionLabel}
      </Button>
    </div>
  );
}
