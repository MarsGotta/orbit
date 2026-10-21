import type { AppView as AppSnapshot } from "../../state/app-view";

import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { CalendarVisibilitySelector } from "./CalendarVisibilitySelector";
import { dayOf, monthTitle, weekNumber } from "./date";
import { includes, type Range } from "./range";
import type { CalendarRangeControls } from "./useCalendarRange";
import { ViewMenu } from "./ViewMenu";
import styles from "./CalendarView.module.css";

type Calendar = AppSnapshot["calendar"];

interface CalendarToolbarProps {
  calendar: Calendar;
  controls: CalendarRangeControls;
  hiddenCalendars: ReadonlySet<string>;
  nowMs: number;
  onShowAll: () => void;
  onStartMeeting: () => void;
  onToggleCalendar: (id: string) => void;
  timezone: string;
}

/** The day that names a period: its middle for a grid, the anchor otherwise. */
function namingDay(range: Range, timezone: string): number {
  if (range.view === "month" || range.view === "agenda") return dayOf(range.anchor_ms, timezone);
  return dayOf(range.start_ms, timezone) + Math.floor((range.days - 1) / 2);
}

function stepLabels(range: Range): [string, string] {
  switch (range.view) {
    case "day":
      return [es.calendar.previous.day, es.calendar.next.day];
    case "days":
      return [es.calendar.previous.days(range.days), es.calendar.next.days(range.days)];
    case "month":
      return [es.calendar.previous.month, es.calendar.next.month];
    case "week":
    case "agenda":
      return [es.calendar.previousWeek, es.calendar.nextWeek];
  }
}

/** «Octubre 2026 · S42», ‹ ›, «Hoy» when today is out of sight, the view. */
export function CalendarToolbar(props: CalendarToolbarProps) {
  const range = props.calendar.range;
  const day = namingDay(range, props.timezone);
  const [previous, next] = stepLabels(range);
  const grid = range.view === "day" || range.view === "days" || range.view === "week";
  return (
    <header className={styles.toolbar}>
      <strong className={styles.month}>{monthTitle(day)}</strong>
      {grid && (
        <>
          <span aria-hidden="true" className={styles.separator}>
            ·
          </span>
          <span className={styles.weekNumber}>{es.calendar.weekNumber(weekNumber(day))}</span>
        </>
      )}
      <div className={styles.weekActions}>
        <Button aria-label={previous} onClick={() => props.controls.step(-1)} size={26}>
          <Chevron direction="left" />
        </Button>
        <Button aria-label={next} onClick={() => props.controls.step(1)} size={26}>
          <Chevron direction="right" />
        </Button>
        {!includes(range, props.nowMs) && (
          <Button onClick={props.controls.today} size={26} variant="ghost">
            {es.calendar.today}
          </Button>
        )}
      </div>
      <div className={styles.toolbarSpacer} />
      <ViewMenu controls={props.controls} range={range} />
      <CalendarVisibilitySelector
        calendars={props.calendar.calendars}
        hidden={props.hiddenCalendars}
        onShowAll={props.onShowAll}
        onToggle={props.onToggleCalendar}
      />
      <Button onClick={props.onStartMeeting}>{es.calendar.meet}</Button>
    </header>
  );
}

function Chevron({ direction }: { direction: "left" | "right" }) {
  return (
    <svg
      aria-hidden="true"
      fill="none"
      height="12"
      stroke="currentColor"
      strokeWidth="2"
      viewBox="0 0 24 24"
      width="12"
    >
      <path d={direction === "left" ? "M15 6l-6 6 6 6" : "M9 6l6 6-6 6"} />
    </svg>
  );
}
