import { useState } from "react";
import { parseTime } from "./inputs";
import styles from "./EventPanel.module.css";

interface TimeFieldProps {
  label: string;
  /** Minutes of the day, as written back once the field is left. */
  onCommit: (minutes: number) => void;
  /** «14:00»: Orbit's 24-hour clock, whatever the system's locale. */
  value: string;
}

/**
 * An hour typed as text: «9», «9:30» or «0930». It is read when
 * the field is left or with ↵; something that is not an hour puts the
 * previous value back (P9-22).
 */
export function TimeField({ label, onCommit, value }: TimeFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft === null) return;
    const minutes = parseTime(draft);
    setDraft(null);
    if (minutes !== null) onCommit(minutes);
  };
  return (
    <input
      aria-label={label}
      className={styles.time}
      inputMode="numeric"
      onBlur={commit}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") commit();
      }}
      value={draft ?? value}
    />
  );
}
