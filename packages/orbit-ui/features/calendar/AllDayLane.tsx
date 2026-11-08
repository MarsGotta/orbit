import { useCopy } from "../../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { type CSSProperties } from "react";

import { dayOf } from "./date";
import { type LaneEdit, useLaneGestures, withLanePreview } from "./edit/useLaneGestures";
import { type CalendarEvent, toneOf, toneStyle } from "./eventLook";
import { type AllDayBar, allDayLayout } from "./layout";
import styles from "./TimeGrid.module.css";

interface AllDayLaneProps {
  calendars: AppSnapshot["calendar"]["calendars"];
  days: number[];
  /** Drafting by days, moving and stretching bars; absent, read only. */
  edit?: LaneEdit;
  events: CalendarEvent[];
  onOpenEvent: (event: CalendarEvent) => void;
  onToggle: () => void;
  open: boolean;
  /** The event whose panel is open. */
  selectedId?: string | null;
  timezone: string;
}

/** The last civil day an event covers: one ending at midnight ends the day before. */
function lastDay(event: CalendarEvent, timezone: string): number {
  return Math.max(dayOf(event.start_ms, timezone), dayOf(event.end_ms - 1, timezone));
}

/**
 * All-day and multi-day events as bars across the columns. Folded
 * it shows one row and says how many bars it is not showing. With editing
 * the lane is always there, so a drag across it drafts a new event.
 */
export function AllDayLane(props: AllDayLaneProps) {
  const es = useCopy();
  const gestures = useLaneGestures(props.edit, props.events, props.days, props.timezone);
  const events = withLanePreview(props.events, gestures.preview, es.calendar.edit.newEvent);
  const byId = new Map(events.map((event) => [event.id, event]));
  const bars = allDayLayout(
    events.map((event) => ({
      firstDay: dayOf(event.start_ms, props.timezone),
      id: event.id,
      lastDay: lastDay(event, props.timezone),
    })),
    props.days,
  );
  if (bars.length === 0 && props.edit === undefined) return null;
  const shown = props.open ? bars : bars.filter((bar) => bar.row === 0);
  const hidden = bars.length - shown.length;
  const columns = { "--columns": props.days.length } as CSSProperties;
  return (
    <div className={styles.allDay} role="row" style={columns}>
      <div className={styles.allDayRail}>
        <button
          aria-expanded={props.open}
          aria-label={props.open ? es.calendar.collapseAllDay : es.calendar.expandAllDay}
          className={styles.allDayToggle}
          disabled={!props.open && hidden === 0}
          onClick={props.onToggle}
          type="button"
        >
          {hidden > 0 ? es.calendar.moreEvents(hidden) : es.calendar.allDayLane}
        </button>
      </div>
      <div
        className={styles.allDayBars}
        data-lane="all-day"
        onMouseDown={gestures.onMouseDown}
        ref={gestures.laneRef}
      >
        {shown.map((bar) => {
          const event = byId.get(bar.id);
          if (!event) return null;
          return <Bar bar={bar} event={event} key={bar.id} {...props} />;
        })}
      </div>
    </div>
  );
}

function Bar(props: AllDayLaneProps & { bar: AllDayBar; event: CalendarEvent }) {
  const es = useCopy();
  const { bar, event } = props;
  const sides =
    props.edit !== undefined &&
    event.all_day === true &&
    event.ui?.saving !== true &&
    props.edit.editable(event);
  return (
    <button
      aria-busy={event.ui?.saving === true ? true : undefined}
      aria-pressed={props.selectedId === event.id}
      className={styles.allDayBar}
      data-clipped-end={bar.clippedEnd ? "true" : undefined}
      data-clipped-start={bar.clippedStart ? "true" : undefined}
      data-draft={event.ui?.draft === true ? "true" : undefined}
      data-dragging={event.ui?.dragging === true ? "true" : undefined}
      data-event-id={event.id}
      data-saving={event.ui?.saving === true ? "true" : undefined}
      data-selected={props.selectedId === event.id ? "true" : undefined}
      onClick={() => props.onOpenEvent(event)}
      style={{
        ...toneStyle(toneOf(event, props.calendars)),
        gridColumn: `${bar.column + 1} / span ${bar.span}`,
        gridRow: bar.row + 1,
      }}
      type="button"
    >
      {sides && !bar.clippedStart && (
        <span
          aria-hidden="true"
          className={styles.barSide}
          data-resize="start"
          title={es.calendar.edit.resizeFirstDay}
        />
      )}
      <span aria-hidden="true" className={styles.barDot} />
      <span className={styles.barTitle}>
        {event.ui?.saving === true ? `${event.title} · ${es.calendar.write.saving}` : event.title}
      </span>
      {sides && !bar.clippedEnd && (
        <span
          aria-hidden="true"
          className={styles.barSide}
          data-resize="end"
          title={es.calendar.edit.resizeLastDay}
        />
      )}
    </button>
  );
}
