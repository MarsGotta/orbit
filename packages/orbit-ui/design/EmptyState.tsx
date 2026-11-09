import type { ReactNode } from "react";
import { Icon, type IconName } from "./Icon";
import styles from "./EmptyState.module.css";

export interface EmptyStateProps {
  icon: IconName;
  title: string;
  children: ReactNode;
  /** The one main action that fills it. */
  action?: ReactNode;
}

/** «Aún no hay notas · Graba tu primera reunión y Orbit escribirá la nota por ti». */
export function EmptyState({ icon, title, children, action }: EmptyStateProps) {
  return (
    <div className={styles.empty}>
      <span className={styles.icon}>
        <Icon name={icon} />
      </span>
      <h3 className={styles.title}>{title}</h3>
      <p className={styles.text}>{children}</p>
      {action}
    </div>
  );
}
