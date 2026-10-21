import type { AppView as AppSnapshot } from "../../state/app-view";

import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import styles from "./CalendarView.module.css";

interface CalendarFeedbackProps {
  calendar: AppSnapshot["calendar"];
  error: string | null;
  onRetry: () => void;
  pending: boolean;
}

/**
 * What the calendar says above its grid: a read on
 * its way, or a read that failed. With the agenda read, nothing (SIM-24).
 */
export function CalendarFeedback(props: CalendarFeedbackProps) {
  const { calendar, error } = props;
  return (
    <>
      {(props.pending || calendar.stale) && error === null && (
        <div className={styles.stale}>{es.calendar.stale}</div>
      )}
      {Boolean(error) && (
        <div className={styles.calendarError} role="alert">
          <span>{error}</span>
          <Button onClick={props.onRetry} size={26}>
            {es.calendar.retry}
          </Button>
        </div>
      )}
    </>
  );
}
