import type { ReactNode } from "react";
import styles from "./AttentionRow.module.css";

export interface AttentionRowProps {
  title: string;
  help?: string;
  /** The button that fixes it: «Abrir Ajustes del Sistema». */
  action?: ReactNode;
}

/** What is missing inside a section: a permission, the calendar not connected. */
export function AttentionRow({ title, help, action }: AttentionRowProps) {
  return (
    <div className={styles.row}>
      <span aria-hidden="true" className={styles.dot} />
      <div className={styles.text}>
        <span className={styles.title}>{title}</span>
        {help && <span className={styles.help}>{help}</span>}
      </div>
      {action}
    </div>
  );
}
