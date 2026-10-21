import type { ReactNode } from "react";
import styles from "./Pill.module.css";

export type PillStatus = "connected" | "disconnected" | "privacy" | "recording";

export function Pill({
  children,
  status = "connected",
}: {
  children: ReactNode;
  status?: PillStatus;
}) {
  return (
    <span className={`${styles.pill} ${styles[status] ?? ""}`} role="status">
      <span aria-hidden="true" className={styles.dot} />
      {children}
    </span>
  );
}
