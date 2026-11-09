import type { CSSProperties } from "react";
import styles from "./CalendarEvent.module.css";

export interface CalendarEventProps {
  /** Which of the four calendar colours: its tint fills it, its dot marks it. */
  color: 1 | 2 | 3 | 4;
  title: string;
  /** «11:00 – 11:45». */
  time: string;
  /** Its height in the grid, in px; without it (a list) nothing is cut. */
  height?: number;
  /** «Quizá» or «Sin responder»: a dashed border and this label under it. */
  response?: { kind: "tentative" | "unanswered"; label: string };
  selected?: boolean;
  past?: boolean;
  onOpen?: () => void;
}

// Title lines are 12 px at 1.35 and the event has 4 px above and below.
const LINE_PX = 16;
const PADDING_PX = 8;

/**
 * How many lines fit: a 30-minute event shows one line of title cut with «…»;
 * the time shows only when two lines fit.
 */
export function eventLines(height: number | undefined, withResponse: boolean) {
  if (height === undefined) return { titleLines: undefined, showTime: true };
  const lines = Math.max(1, Math.floor((height - PADDING_PX) / LINE_PX));
  const showTime = lines >= 2;
  const titleLines = Math.max(1, lines - (showTime ? 1 : 0) - (withResponse ? 1 : 0));
  return { titleLines, showTime };
}

export function CalendarEvent(props: CalendarEventProps) {
  const { color, title, time, height, response, selected, past, onOpen } = props;
  const { titleLines, showTime } = eventLines(height, response !== undefined);
  const style = {
    "--event-color": `var(--o-c${color})`,
    "--event-tint": `var(--o-c${color}s)`,
    "--title-lines": titleLines,
    height,
  } as CSSProperties;
  return (
    <button
      aria-label={`${title}, ${time}${response ? `, ${response.label}` : ""}`}
      aria-pressed={selected}
      className={styles.event}
      data-past={past || undefined}
      data-response={response?.kind}
      onClick={onOpen}
      style={style}
      type="button"
    >
      <span className={styles.titleRow}>
        <span aria-hidden="true" className={styles.dot} />
        <span className={styles.title} data-clamped={titleLines !== undefined || undefined}>
          {title}
        </span>
      </span>
      {showTime && <span className={styles.time}>{time}</span>}
      {response && (
        <span className={styles.response} data-kind={response.kind}>
          {response.label}
        </span>
      )}
    </button>
  );
}
