import type { AppView as AppSnapshot } from "../../state/app-view";
import type { CSSProperties } from "react";
import { es } from "../../i18n/es";
import {
  civil,
  dayHeading,
  dayOf,
  isWeekend,
  rangeDays,
  shortDate,
  startOfDay,
  timeLabel,
} from "./date";
import { EventIcon } from "./EventCard";
import { type CalendarEvent, eventLook, eventsByDay, inAllDayLane, paletteOf } from "./eventLook";
import type { Range } from "./range";
import palette from "./CalendarPalette.module.css";
import styles from "./MonthView.module.css";

/** One 22 px event row of a day cell. */
export const MONTH_ROW_PX = 22;
/** The day number above a cell's rows. */
const CELL_HEADER_PX = 28;
/** Rows a cell holds before its size is measured. */
const UNMEASURED_ROWS = 3;

interface MonthViewProps {
  calendars: AppSnapshot["calendar"]["calendars"];
  events: CalendarEvent[];
  /** The height the weeks have to share; 0 before it is measured. */
  height: number;
  nowMs: number;
  /** A double click in a day drafts an all-day event there. */
  onDraftDay?: (day: number) => void;
  onOpenDay: (dayStartMs: number) => void;
  onOpenEvent: (event: CalendarEvent) => void;
  range: Range;
  showWeekends: boolean;
  timezone: string;
}

/** The month's weeks whole, one row per event in each day. */
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
      ? Math.max(1, Math.floor((cellHeight - CELL_HEADER_PX) / MONTH_ROW_PX))
      : UNMEASURED_ROWS;
  const today = dayOf(props.nowMs, props.timezone);
  const columns = { "--columns": weeks[0]?.length ?? 7 } as CSSProperties;
  return (
    <div aria-label={es.calendar.month} className={styles.month} role="grid" style={columns}>
      <div className={styles.weekdays} role="row">
        {(weeks[0] ?? []).map((day) => (
          <div key={day} role="columnheader">
            {dayHeading(day).name}
          </div>
        ))}
      </div>
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
  const shown = overflow ? props.events.slice(0, props.rows - 1) : props.events;
  const label = shortDate(props.day);
  const openDay = () => props.onOpenDay(startOfDay(props.day, props.timezone));
  return (
    <div
      aria-label={`${label.number} ${label.month}`}
      className={[styles.cell, props.inMonth ? "" : styles.outside].join(" ")}
      onDoubleClick={(event) => {
        if ((event.target as HTMLElement).closest("button") === null) props.onDraftDay?.(props.day);
      }}
      role="gridcell"
    >
      <button
        className={props.isToday ? styles.todayNumber : styles.number}
        onClick={openDay}
        type="button"
      >
        {label.number}
      </button>
      {shown.map((event) => (
        <MonthRow {...props} event={event} key={event.id} />
      ))}
      {overflow && (
        <button className={styles.more} onClick={openDay} type="button">
          {es.calendar.moreEvents(props.events.length - shown.length)}
        </button>
      )}
    </div>
  );
}

function MonthRow(props: MonthCellProps & { event: CalendarEvent }) {
  const { event } = props;
  const look = eventLook(event, props.nowMs);
  const allDay = inAllDayLane(event);
  return (
    <button
      className={[
        allDay ? styles.bar : styles.row,
        palette[paletteOf(event, props.calendars)] ?? "",
        look.striped ? styles.striped : "",
      ].join(" ")}
      data-response={event.self_response ?? undefined}
      onClick={() => props.onOpenEvent(event)}
      style={{
        opacity: look.done || look.declined ? 0.5 : undefined,
        textDecorationLine: look.done || look.declined ? "line-through" : undefined,
      }}
      type="button"
    >
      <span className={styles.title}>{event.title}</span>
      <EventIcon look={look} />
      {!allDay && <time>{timeLabel(event.start_ms, props.timezone)}</time>}
    </button>
  );
}
