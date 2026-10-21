import type { AppView as AppSnapshot } from "../../state/app-view";
import { es } from "../../i18n/es";
import { dayOf, rangeDays, shortDate, startOfDay, timeLabel } from "./date";
import { type CalendarEvent, eventsByDay, inAllDayLane, paletteOf } from "./eventLook";
import type { Range } from "./range";
import palette from "./CalendarPalette.module.css";
import styles from "./AgendaList.module.css";

interface AgendaListProps {
  calendars: AppSnapshot["calendar"]["calendars"];
  events: CalendarEvent[];
  nowMs: number;
  onOpenDay: (dayStartMs: number) => void;
  onOpenEvent: (event: CalendarEvent) => void;
  range: Range;
  /** While the read is not complete, an empty today is not «nothing». */
  stale: boolean;
  timezone: string;
}

/**
 * The agenda (P9-20): days with events, 31 either side of the day asked
 * for. Empty days are left out, except today.
 */
export function AgendaList(props: AgendaListProps) {
  const days = rangeDays(props.range, props.timezone);
  const today = dayOf(props.nowMs, props.timezone);
  const byDay = eventsByDay(props.events, days, props.timezone);
  const shown = days.filter((day) => (byDay.get(day)?.length ?? 0) > 0 || day === today);
  return (
    <ol aria-label={es.calendar.agenda} className={styles.agenda}>
      {shown.map((day) => (
        <AgendaDay
          {...props}
          day={day}
          events={byDay.get(day) ?? []}
          isToday={day === today}
          key={day}
          past={day < today}
        />
      ))}
    </ol>
  );
}

function AgendaDay(props: AgendaListProps & { day: number; isToday: boolean; past: boolean }) {
  const date = shortDate(props.day);
  return (
    <li className={[styles.day, props.past ? styles.past : ""].join(" ")}>
      <button
        className={styles.date}
        onClick={() => props.onOpenDay(startOfDay(props.day, props.timezone))}
        type="button"
      >
        {props.isToday ? (
          <span className={styles.todayPill}>{es.calendar.agendaToday}</span>
        ) : (
          <>
            <strong>{date.number}</strong>
            <span>{date.month}</span>
          </>
        )}
      </button>
      <div className={styles.rows}>
        {props.events.length === 0 && !props.stale && (
          <p className={styles.free}>{es.calendar.freeToday}</p>
        )}
        {props.events.map((event) => (
          <AgendaRow {...props} event={event} key={event.id} />
        ))}
      </div>
    </li>
  );
}

function AgendaRow(props: AgendaListProps & { event: CalendarEvent }) {
  const { event } = props;
  const guests = event.attendees?.length ?? 0;
  const over = event.end_ms <= props.nowMs;
  return (
    <button
      className={[
        styles.row,
        palette[paletteOf(event, props.calendars)] ?? "",
        over ? styles.over : "",
      ].join(" ")}
      data-response={event.self_response ?? undefined}
      onClick={() => props.onOpenEvent(event)}
      style={{
        textDecorationLine: event.self_response === "declined" ? "line-through" : undefined,
      }}
      type="button"
    >
      <span aria-hidden="true" className={styles.stripe} />
      <span className={styles.hours}>
        {inAllDayLane(event)
          ? es.calendar.allDay
          : es.calendar.timeRange(
              timeLabel(event.start_ms, props.timezone),
              timeLabel(event.end_ms, props.timezone),
            )}
      </span>
      <span className={styles.title}>{event.title}</span>
      {guests > 0 && (
        <span aria-label={es.calendar.guests(guests)} className={styles.meta} role="img">
          <PeopleGlyph />
          {guests}
        </span>
      )}
      {event.conference_url != null && (
        <span aria-label={es.calendar.videoCall} className={styles.meta} role="img">
          <VideoGlyph />
        </span>
      )}
    </button>
  );
}

function PeopleGlyph() {
  return (
    <svg
      aria-hidden="true"
      fill="none"
      height="12"
      stroke="currentColor"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
      width="12"
    >
      <circle cx="9" cy="8.5" r="3.5" />
      <path d="M3 20a6 6 0 0 1 12 0M16 5.2a3.5 3.5 0 0 1 0 6.6M17.5 20a6 6 0 0 0-2-4.5" />
    </svg>
  );
}

export function VideoGlyph() {
  return (
    <svg
      aria-hidden="true"
      fill="none"
      height="12"
      stroke="currentColor"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
      width="12"
    >
      <rect height="12" rx="2.5" width="13" x="3" y="6" />
      <path d="M16 10.5l5-3v9l-5-3z" />
    </svg>
  );
}
