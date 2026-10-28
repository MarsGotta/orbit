import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { Icon } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";

import {
  civil,
  dayName,
  dayOf,
  rangeDays,
  shortDate,
  startOfDay,
  timeLabel,
  weekdayName,
} from "./date";
import { type CalendarEvent, eventsByDay, inAllDayLane, toneOf, toneStyle } from "./eventLook";
import type { Range } from "./range";
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

const LOADING_ROWS = ["60%", "45%"];

/**
 * The agenda: the days with events, 31 either side of the day asked for, in a
 * list grouped by day. Empty days are left out, except today, marked «HOY».
 */
export function AgendaList(props: AgendaListProps) {
  const days = rangeDays(props.range, props.timezone);
  const today = dayOf(props.nowMs, props.timezone);
  const byDay = eventsByDay(props.events, days, props.timezone);
  const shown = days.filter((day) => (byDay.get(day)?.length ?? 0) > 0 || day === today);
  return (
    <div className={styles.scroll}>
      <div className={styles.agenda}>
        {shown.map((day) => (
          <AgendaDay
            {...props}
            day={day}
            events={byDay.get(day) ?? []}
            isToday={day === today}
            key={day}
          />
        ))}
      </div>
    </div>
  );
}

function AgendaDay(props: AgendaListProps & { day: number; isToday: boolean }) {
  const es = useCopy();
  return (
    <section aria-label={dayName(props.day)} className={styles.day}>
      <button
        className={styles.date}
        data-today={props.isToday ? "true" : undefined}
        onClick={() => props.onOpenDay(startOfDay(props.day, props.timezone))}
        type="button"
      >
        <span className={styles.number}>{civil(props.day).date}</span>
        <span className={styles.weekday}>
          {`${weekdayName(props.day)} · ${shortDate(props.day).month}`}
        </span>
        {props.isToday && <span className={styles.today}>{es.calendar.agendaToday}</span>}
      </button>
      <div className={styles.rows}>
        {props.events.length === 0 &&
          (props.stale ? (
            <div className={styles.loading}>
              <Skeleton height={11} widths={LOADING_ROWS} />
            </div>
          ) : (
            <p className={styles.free}>{es.calendar.freeToday}</p>
          ))}
        {props.events.map((event) => (
          <AgendaRow {...props} event={event} key={event.id} />
        ))}
      </div>
    </section>
  );
}

/** The line under a title, when it needs one: the answer, the place, how long. */
function secondLine(
  event: CalendarEvent,
  day: number,
  timezone: string,
): { text: string; warn: boolean } | null {
  if (event.self_response === "needs_action")
    return { text: getCopy().calendar.unansweredHere, warn: true };
  if (event.self_response === "tentative")
    return { text: getCopy().calendar.selfResponse.tentative, warn: false };
  if (event.location != null && event.location !== "") return { text: event.location, warn: false };
  const last = dayOf(event.end_ms - 1, timezone);
  if (inAllDayLane(event) && day === dayOf(event.start_ms, timezone) && last > day)
    return { text: getCopy().calendar.untilDay(weekdayName(last)), warn: false };
  return null;
}

function AgendaRow(props: AgendaListProps & { day: number; event: CalendarEvent }) {
  const es = useCopy();
  const { event } = props;
  const guests = event.attendees?.length ?? 0;
  const sub = secondLine(event, props.day, props.timezone);
  return (
    <button
      className={styles.row}
      data-declined={event.self_response === "declined" ? "true" : undefined}
      data-past={event.end_ms <= props.nowMs ? "true" : undefined}
      onClick={() => props.onOpenEvent(event)}
      style={toneStyle(toneOf(event, props.calendars))}
      type="button"
    >
      <span aria-hidden="true" className={styles.dot} />
      <span className={styles.hours}>
        {inAllDayLane(event)
          ? es.calendar.allDay
          : es.calendar.timeRange(
              timeLabel(event.start_ms, props.timezone),
              timeLabel(event.end_ms, props.timezone),
            )}
      </span>
      <span className={styles.text}>
        <span className={styles.title}>{event.title}</span>
        {sub !== null && (
          <span className={styles.sub} data-warn={sub.warn ? "true" : undefined}>
            {sub.text}
          </span>
        )}
      </span>
      {guests > 0 && (
        <span aria-label={es.calendar.guests(guests)} className={styles.meta} role="img">
          <Icon name="people" size={14} />
          {guests}
        </span>
      )}
      {event.conference_url != null && (
        <span className={styles.meta}>
          <Icon name="video" size={14} title={es.calendar.videoCall} />
        </span>
      )}
    </button>
  );
}
