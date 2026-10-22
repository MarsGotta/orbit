import type { AppView as AppSnapshot } from "../../state/app-view";
import { type CSSProperties, useEffect, useMemo, useRef } from "react";
import { es } from "../../i18n/es";
import { AllDayLane } from "./AllDayLane";
import { DayColumn, type DaySegment } from "./DayColumn";
import {
  dayHeading,
  dayOf,
  eventMinutes,
  eventSegments,
  hourLabel,
  isWeekend,
  rangeDays,
  timeLabel,
  timezoneCity,
} from "./date";
import { ghostSegments } from "./edit/ghost";
import type { LaneEdit } from "./edit/useLaneGestures";
import { type GridEdit, useGridGestures } from "./edit/useGridGestures";
import { type CalendarEvent, inAllDayLane } from "./eventLook";
import type { Range } from "./range";
import styles from "./TimeGrid.module.css";

export interface TimeGridProps {
  allDayOpen: boolean;
  /** Create, move and stretch with the mouse; absent, read only. */
  edit?: GridEdit;
  /** The all-day lane's own gestures. */
  laneEdit?: LaneEdit;
  calendars: AppSnapshot["calendar"]["calendars"];
  events: CalendarEvent[];
  hourPx: number;
  /** The period's events are on their way: the columns show where they will be. */
  loading?: boolean;
  nowMs: number;
  onOpenEvent: (event: CalendarEvent) => void;
  onToggleAllDay: () => void;
  range: Range;
  /** The event whose panel is open. */
  selectedId?: string | null;
  showWeekends: boolean;
  timezone: string;
}

/** The days a grid paints: the range, without weekends when the user hides them
 *  in a view a week wide or wider. A day view always shows its day. */
export function gridDays(range: Range, timezone: string, showWeekends: boolean): number[] {
  const days = rangeDays(range, timezone);
  return showWeekends || range.days < 7 ? days : days.filter((day) => !isWeekend(day));
}

export function gridLabel(range: Range): string {
  if (range.view === "day") return es.calendar.day;
  if (range.view === "days") return es.calendar.days(range.days);
  return es.calendar.week;
}

/** The timed events of each day in view, cut at midnight. */
function segmentsByDay(events: readonly CalendarEvent[], days: number[], timezone: string) {
  const byDay = new Map<number, DaySegment[]>();
  for (const event of events.filter((item) => !inAllDayLane(item))) {
    for (const segment of eventSegments(event.start_ms, event.end_ms, days, timezone)) {
      byDay.set(segment.day, [...(byDay.get(segment.day) ?? []), { ...segment, event }]);
    }
  }
  return byDay;
}

/** `segmentsByDay`, recomputed only when the events, the days or the zone change. */
function useDaySegments(events: readonly CalendarEvent[], days: number[], timezone: string) {
  const dayKey = days.join(",");
  return useMemo(
    () => segmentsByDay(events, dayKey === "" ? [] : dayKey.split(",").map(Number), timezone),
    [events, dayKey, timezone],
  );
}

/** On opening, the present less fifteen minutes is at the top. Only
 *  on opening: the clock ticking must not move what the user scrolled to. */
function useOpenAtNow(nowMs: number, timezone: string, hourPx: number) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const opening = useRef({ hourPx, nowMs, timezone });
  useEffect(() => {
    const { hourPx: px, nowMs: now, timezone: zone } = opening.current;
    if (scrollRef.current) scrollRef.current.scrollTop = ((eventMinutes(now, zone) - 15) / 60) * px;
  }, []);
  return scrollRef;
}

/** Day, N days and week: 24 hours, an all-day lane and the present. */
export function TimeGrid(props: TimeGridProps) {
  const scrollRef = useOpenAtNow(props.nowMs, props.timezone, props.hourPx);
  const days = gridDays(props.range, props.timezone, props.showWeekends);
  const today = dayOf(props.nowMs, props.timezone);
  const columns = { "--columns": days.length } as CSSProperties;
  return (
    <div
      aria-busy={props.loading === true ? true : undefined}
      aria-label={gridLabel(props.range)}
      className={styles.grid}
      role="grid"
      style={columns}
    >
      <div className={styles.dayHeader} role="row">
        <div className={styles.timezone}>{timezoneCity(props.timezone)}</div>
        {days.map((day) => (
          <DayHeading day={day} isToday={day === today} key={day} />
        ))}
      </div>
      <AllDayLane
        calendars={props.calendars}
        days={days}
        edit={props.laneEdit}
        events={props.events.filter(inAllDayLane)}
        onOpenEvent={props.onOpenEvent}
        onToggle={props.onToggleAllDay}
        open={props.allDayOpen}
        selectedId={props.selectedId}
        timezone={props.timezone}
      />
      <div className={styles.gridScroll} ref={scrollRef}>
        <GridBody {...props} days={days} today={today} />
      </div>
    </div>
  );
}

/** The 24 hours of every day in view, where the gestures happen. */
function GridBody(props: TimeGridProps & { days: number[]; today: number }) {
  const { days, today } = props;
  const nowMinutes = days.includes(today) ? eventMinutes(props.nowMs, props.timezone) : null;
  const segments = useDaySegments(props.events, days, props.timezone);
  const timed = props.events.filter((event) => !inAllDayLane(event));
  const gestures = useGridGestures(props.edit, timed, props.hourPx, props.timezone);
  const ghosts = ghostSegments(gestures.preview, timed, days, props.timezone);
  return (
    <div
      className={styles.gridBody}
      data-editing={gestures.preview !== null ? "true" : undefined}
      onMouseDown={gestures.onMouseDown}
      ref={gestures.bodyRef}
      style={{ height: 24 * props.hourPx }}
    >
      <HourRail
        hourPx={props.hourPx}
        nowMs={props.nowMs}
        nowMinutes={nowMinutes}
        timezone={props.timezone}
      />
      {days.map((day) => (
        <DayColumn
          calendars={props.calendars}
          day={day}
          draggingId={gestures.preview?.eventId ?? null}
          editable={props.edit?.editable}
          ghosts={ghosts.get(day) ?? []}
          hourPx={props.hourPx}
          isToday={day === today}
          key={day}
          nowMinutes={nowMinutes}
          nowMs={props.nowMs}
          onOpenEvent={props.onOpenEvent}
          placeholder={props.loading === true && timed.length === 0}
          segments={segments.get(day) ?? []}
          selectedId={props.selectedId}
          timezone={props.timezone}
        />
      ))}
    </div>
  );
}

function DayHeading({ day, isToday }: { day: number; isToday: boolean }) {
  const heading = dayHeading(day);
  return (
    <div
      aria-current={isToday ? "date" : undefined}
      className={styles.dayHeading}
      role="columnheader"
    >
      <span>{heading.name}</span>
      <span className={isToday ? styles.todayPill : styles.dayNumber}>{heading.number}</span>
    </div>
  );
}

const HOURS = Array.from({ length: 23 }, (_, index) => index + 1);

function HourRail(props: {
  hourPx: number;
  nowMinutes: number | null;
  nowMs: number;
  timezone: string;
}) {
  return (
    <div aria-hidden="true" className={styles.timeRail}>
      {HOURS.map((hour) => (
        <span key={hour} style={{ top: hour * props.hourPx - 7 }}>
          {hourLabel(hour)}
        </span>
      ))}
      {props.nowMinutes !== null && (
        <span
          className={styles.nowLabel}
          style={{ top: (props.nowMinutes / 60) * props.hourPx - 7 }}
        >
          {timeLabel(props.nowMs, props.timezone)}
        </span>
      )}
    </div>
  );
}
