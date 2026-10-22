import type { ReactNode } from "react";
import { Spinner } from "./Spinner";
import styles from "./StatusPill.module.css";

/**
 * `recording` «Grabando · 12:34» with the beating dot; `working` «Transcribiendo…»
 * with the spinner; `done` «✓ Nota lista»; `warning` «Falta un permiso».
 * Colour only when something happens.
 */
export type StatusKind = "recording" | "working" | "done" | "warning";

export function StatusPill({ children, kind }: { children: ReactNode; kind: StatusKind }) {
  return (
    <span className={`${styles.pill} ${styles[kind]}`} role="status">
      {kind === "recording" && <span aria-hidden="true" className={styles.dot} />}
      {kind === "working" && <Spinner size={12} />}
      {children}
    </span>
  );
}
