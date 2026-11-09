import type { ReactNode } from "react";
import { Skeleton } from "../../design/Skeleton";
import styles from "./SettingsView.module.css";

/** The section's title and the sentence that says what it holds. */
export function SectionHeading({ title, lead }: { title: string; lead: string }) {
  return (
    <header className={styles.heading}>
      <h2>{title}</h2>
      <p>{lead}</p>
    </header>
  );
}

/**
 * One part of a section: its name above, with a count of what fits beside it
 * («3 de 8») and a line of help below, then its card.
 */
export function Group({
  label,
  count,
  help,
  children,
}: {
  label: string;
  count?: string;
  help?: string;
  children: ReactNode;
}) {
  return (
    <section aria-label={label} className={styles.group}>
      <div className={styles.groupLabel}>
        <h3>{label}</h3>
        {count !== undefined && <span className={styles.count}>{count}</span>}
      </div>
      {help !== undefined && <p className={styles.groupHelp}>{help}</p>}
      {children}
    </section>
  );
}

/** A card of rows, each separated from the next by a line. */
export function List({ children, busy }: { children: ReactNode; busy?: boolean }) {
  return (
    <div aria-busy={busy === true ? true : undefined} className={styles.list}>
      {children}
    </div>
  );
}

/**
 * One row: what it is and what it is for, what it says at the right, and
 * whatever it holds below. `warn` is something missing (the attention row).
 */
export function Row({
  title,
  help,
  aside,
  children,
  warn,
}: {
  title: ReactNode;
  help?: ReactNode;
  aside?: ReactNode;
  children?: ReactNode;
  warn?: boolean;
}) {
  return (
    <div className={styles.row} data-warn={warn === true ? true : undefined}>
      <div className={styles.rowText}>
        <div className={styles.rowTitle}>{title}</div>
        {help !== undefined && <div className={styles.rowHelp}>{help}</div>}
        {children}
      </div>
      {aside}
    </div>
  );
}

/** Rows still coming: the shape of what will arrive, never «none». */
export function SkeletonRows({ rows = 3 }: { rows?: number }) {
  return (
    <List busy>
      {Array.from({ length: rows }, (_, index) => (
        <div className={styles.row} key={index}>
          <Skeleton widths={["32%", "64%"]} />
        </div>
      ))}
    </List>
  );
}
