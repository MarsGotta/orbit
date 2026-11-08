import { useCopy } from "../../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { Button } from "../../design/Button";
import { Skeleton } from "../../design/Skeleton";

import { dayAndMonth, dayOf, longDay, startOfDay, timeLabel, weekdayName } from "./date";
import { type CalendarEvent, eventsByDay, inAllDayLane, toneOf, toneStyle } from "./eventLook";
import styles from "./DayAgenda.module.css";

interface DayAgendaProps {
  calendars: AppSnapshot["calendar"]["calendars"];
  day: number;
  events: CalendarEvent[];
  loading: boolean;
  nowMs: number;
  onOpenDay: (dayStartMs: number) => void;
  /** Opens the event in its day, with its panel. */
  onOpenEvent: (event: CalendarEvent) => void;
  timezone: string;
}

const LOADING_ROWS = ["70%", "55%", "62%"];

/**
 * The day picked in the month, on its right: «Today · Miércoles, 21 de
 * octubre», its events in order and «Abrir este día».
 */
export function DayAgenda(props: DayAgendaProps) {
  const es = useCopy();
  const today = props.day === dayOf(props.nowMs, props.timezone);
  const weekday = weekdayName(props.day);
  const events = eventsByDay(props.events, [props.day], props.timezone).get(props.day) ?? [];
  return (
    <aside aria-label={es.calendar.chosenDay} className={styles.day}>
      <p className={styles.over}>
        {today ? es.calendar.today : weekday.charAt(0).toUpperCase() + weekday.slice(1)}
      </p>
      <h3 className={styles.title}>{today ? longDay(props.day) : dayAndMonth(props.day)}</h3>
      {props.loading && events.length === 0 ? (
        <Skeleton height={11} widths={LOADING_ROWS} />
      ) : events.length === 0 ? (
        <p className={styles.empty}>{es.calendar.nothingThisDay}</p>
      ) : (
        <ul className={styles.list}>
          {events.map((event) => (
            <li key={event.id}>
              <button
                className={styles.row}
                data-past={event.end_ms <= props.nowMs ? "true" : undefined}
                onClick={() => props.onOpenEvent(event)}
                style={toneStyle(toneOf(event, props.calendars))}
                type="button"
              >
                <span className={styles.time}>
                  {inAllDayLane(event)
                    ? es.calendar.allDay
                    : timeLabel(event.start_ms, props.timezone)}
                </span>
                <span aria-hidden="true" className={styles.dot} />
                <span className={styles.text}>
                  {event.title}
                  {event.self_response === "needs_action" && (
                    <span className={styles.unanswered}>{es.calendar.unanswered}</span>
                  )}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className={styles.spacer} />
      <Button
        className={styles.open}
        onClick={() => props.onOpenDay(startOfDay(props.day, props.timezone))}
        size={36}
      >
        {es.calendar.openDay}
      </Button>
      <p className={styles.hint}>{es.calendar.draftHint}</p>
    </aside>
  );
}
