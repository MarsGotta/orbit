import type { AppView as AppSnapshot } from "../../state/app-view";
import { type CSSProperties, useState } from "react";
import { es } from "../../i18n/es";
import {
  civil,
  dayHeading,
  dayName,
  dayOf,
  isWeekend,
  rangeDays,
  shortDate,
  timeLabel,
} from "./date";
import { type CalendarEvent, eventsByDay, inAllDayLane, toneOf, toneStyle } from "./eventLook";
import type { Range } from "./range";
import styles from "./MonthView.module.css";

/** One event row of a day cell. */
const MONTH_ROW_PX = 22;
/** The day number and the «N más» line around a cell's rows. */
const CELL_CHROME_PX = 50;
/** The design shows at most three events a day, then «N más». */
const MAX_ROWS = 3;

interface MonthViewProps {
  calendars: AppSnapshot["calendar"]["calendars"];
  events: CalendarEvent[];
  /** The height the weeks have to share; 0 before it is measured. */
  height: number;
  /** The period is still being read: the cells fill in. */
  loading: boolean;
  nowMs: number;
  /** A double click in a day drafts an all-day event there. */
  onDraftDay?: (day: number) => void;
  onPickDay: (day: number) => void;
  /** The day whose agenda is on the right. */
  pickedDay: number;
  range: Range;
  showWeekends: boolean;
  timezone: string;
}

/**
 * The day the month shows on the right: today when it is in the month on
 * screen, else the day the month was asked for, until the user picks another.
 * A new month starts again from its own day.
 */
export function usePickedDay(range: Range, nowMs: number, timezone: string) {
  const [picked, setPicked] = useState<{ day: number; startMs: number } | null>(null);
  const anchor = dayOf(range.anchor_ms, timezone);
  const today = dayOf(nowMs, timezone);
  const fallback = civil(today).month === civil(anchor).month ? today : anchor;
  const day = picked !== null && picked.startMs === range.start_ms ? picked.day : fallback;
  const pick = (next: number) => setPicked({ day: next, startMs: range.start_ms });
  return [day, pick] as const;
}

/** Five or six weeks; each day with up to three events, then «N más». */
export function MonthView(props: MonthViewProps) {
  const days = rangeDays(props.range, props.timezone);
  const weeks = Array.from({ length: Math.ceil(days.length / 7) }, (_, index) =>
    days.slice(index * 7, index * 7 + 7).filter((day) => props.showWeekends || !isWeekend(day)),
  );
  const focusMonth = civil(dayOf(props.range.anchor_ms, props.timezone)).month;
  const byDay = eventsByDay(props.events, days, props.timezone);
  const cellHeight = props.height > 0 ? props.height / Math.max(weeks.length, 1) : 0;
  const rows =
    cellHeight > 0
      ? Math.min(MAX_ROWS, Math.max(1, Math.floor((cellHeight - CELL_CHROME_PX) / MONTH_ROW_PX)))
      : MAX_ROWS;
  const today = dayOf(props.nowMs, props.timezone);
  const grid = {
    "--columns": weeks[0]?.length ?? 7,
    "--weeks": weeks.length,
  } as CSSProperties;
  return (
    <div
      aria-busy={props.loading ? true : undefined}
      aria-label={es.calendar.month}
      className={styles.month}
      role="grid"
      style={grid}
    >
      <div className={styles.weekdays} role="row">
        {(weeks[0] ?? []).map((day) => (
          <div key={day} role="columnheader">
            {dayHeading(day).name}
          </div>
        ))}
      </div>
      <div className={styles.weeks}>
        {weeks.map((week) => (
          <div className={styles.week} key={week[0]} role="row">
            {week.map((day) => (
              <MonthCell
                {...props}
                day={day}
                events={byDay.get(day) ?? []}
                inMonth={civil(day).month === focusMonth}
                isToday={day === today}
                key={day}
                rows={rows}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

interface MonthCellProps extends MonthViewProps {
  day: number;
  inMonth: boolean;
  isToday: boolean;
  rows: number;
}

function MonthCell(props: MonthCellProps) {
  const overflow = props.events.length > props.rows;
  const shown = props.events.slice(0, props.rows);
  const number = civil(props.day).date;
  const label = number === 1 ? `1 ${shortDate(props.day).month}` : String(number);
  return (
    <div className={styles.cellWrap} role="gridcell">
      <button
        aria-label={es.calendar.dayCell(dayName(props.day), props.events.length)}
        aria-pressed={props.pickedDay === props.day}
        className={styles.cell}
        data-outside={props.inMonth ? undefined : "true"}
        onClick={() => props.onPickDay(props.day)}
        onDoubleClick={() => props.onDraftDay?.(props.day)}
        type="button"
      >
        <span className={styles.number} data-today={props.isToday ? "true" : undefined}>
          {label}
        </span>
        {props.loading && shown.length === 0 && (
          <span aria-hidden="true" className={styles.placeholder} />
        )}
        {shown.map((event) => (
          <MonthItem {...props} event={event} key={event.id} />
        ))}
        {overflow && (
          <span className={styles.more}>
            {es.calendar.moreEvents(props.events.length - shown.length)}
          </span>
        )}
      </button>
    </div>
  );
}

function MonthItem(props: MonthCellProps & { event: CalendarEvent }) {
  const { event } = props;
  const allDay = inAllDayLane(event);
  return (
    <span
      aria-hidden="true"
      className={allDay ? styles.chip : styles.item}
      data-past={event.end_ms <= props.nowMs ? "true" : undefined}
      data-unanswered={event.self_response === "needs_action" ? "true" : undefined}
      style={toneStyle(toneOf(event, props.calendars))}
    >
      <span className={styles.dot} />
      {!allDay && <span className={styles.time}>{timeLabel(event.start_ms, props.timezone)}</span>}
      <span className={styles.title}>{event.title}</span>
    </span>
  );
}
