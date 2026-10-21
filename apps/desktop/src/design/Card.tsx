import type { ReactNode } from "react";
import styles from "./Card.module.css";

export interface CardProps {
  children: ReactNode;
}

export function Card({ children }: CardProps) {
  return (
    <section className={styles.card}>
      <div className={styles.content}>{children}</div>
    </section>
  );
}
