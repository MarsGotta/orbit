import type { CSSProperties, ReactNode } from "react";
import styles from "./Spinner.module.css";

/** 13 is the progress card's. */
export type SpinnerSize = 12 | 13 | 14 | 16 | 20;

/** «Gira»: something under way that will end by itself. Never alone on screen: see `Loading`. */
export function Spinner({ size = 14, accent = false }: { size?: SpinnerSize; accent?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={`${styles.spinner} ${accent ? styles.accent : ""}`}
      style={{ "--spinner-size": `${size}px` } as CSSProperties}
    />
  );
}

/** The spinner with what is happening beside it: «Leyendo tus notas del hub…». */
export function Loading({ children, size = 16 }: { children: ReactNode; size?: SpinnerSize }) {
  return (
    <span className={styles.loading} role="status">
      <Spinner size={size} />
      {children}
    </span>
  );
}
