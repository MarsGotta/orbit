import type { ReactNode } from "react";
import styles from "./SectionLabel.module.css";

export function SectionLabel({ children }: { children: ReactNode }) {
  return <div className={styles.label}>{children}</div>;
}
