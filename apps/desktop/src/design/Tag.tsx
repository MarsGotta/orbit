import type { ReactNode } from "react";
import styles from "./Tag.module.css";

export function Tag({ children }: { children: ReactNode }) {
  return <span className={styles.tag}>{children}</span>;
}
