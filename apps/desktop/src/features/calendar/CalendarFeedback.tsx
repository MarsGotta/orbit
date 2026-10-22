import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import styles from "./CalendarView.module.css";

interface CalendarFeedbackProps {
  error: string | null;
  onRetry: () => void;
}

/**
 * A read of the calendar that failed, with «Reintentar». A read on its way
 * is told in the header; with the agenda read, nothing.
 */
export function CalendarFeedback({ error, onRetry }: CalendarFeedbackProps) {
  if (error === null || error === "") return null;
  return (
    <div className={styles.alert} role="alert">
      <span aria-hidden="true" className={styles.alertDot} />
      <span className={styles.alertText}>{error}</span>
      <Button onClick={onRetry} size={28}>
        {es.calendar.retry}
      </Button>
    </div>
  );
}
