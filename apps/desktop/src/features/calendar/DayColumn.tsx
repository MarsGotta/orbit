import type { AppView as AppSnapshot } from "../../state/app-view";
import { useMemo } from "react";
import { es } from "../../i18n/es";
import { dayOf, timeLabel } from "./date";
import { EventCard } from "./EventCard";
import { type CalendarEvent, eventLook, toneOf } from "./eventLook";
import { calculateEventBox, layoutDayEvents, type PlacedEvent } from "./layout";
import styles from "./TimeGrid.module.css";

export interface DaySegment {
  endMinutes: number;
  event: CalendarEvent;
  startMinutes: number;
}

interface DayColumnProps {
  calendars: AppSnapshot["calendar"]["calendars"];
  /** The civil day this column is: what a gesture over it reads. */
  day: number;
  /** The card a gesture is moving, drawn dimmed where it was. */
  draggingId?: string | null;
  /** Whether a card may be moved and stretched; absent, none is. */
  editable?: (event: CalendarEvent) => boolean;
  /** The gesture in progress, over the other cards. */
  ghosts?: DaySegment[];
  hourPx: number;
  /** Where the present is on this day, in minutes, when today is in view. */
  nowMinutes: number | null;
  isToday: boolean;
  nowMs: number;
  onOpenEvent: (event: CalendarEvent) => void;
  /** Two blocks where events will be while the period is read. */
  placeholder?: boolean;
  segments: DaySegment[];
  /** The event whose panel is open. */
  selectedId?: string | null;
  timezone: string;
}

const HOUR_LINES = Array.from({ length: 23 }, (_, index) => index + 1);
/** Where the blocks of a period being read sit: a morning and an afternoon. */
const PLACEHOLDER_HOURS: [number, number][] = [
  [9, 1],
  [15, 1.5],
];
/** A ghost covers the column above the cards it passes over. */
const GHOST_PLACE = { depth: 50, leftPercent: 0, widthPercent: 100 };

function usePlacements(segments: DaySegment[]) {
  return useMemo(
    () =>
      new Map(
        layoutDayEvents(
          segments.map(({ endMinutes, event, startMinutes }) => ({
            endMinutes,
            id: event.id,
            startMinutes,
          })),
        ).map((placement) => [placement.id, placement]),
      ),
    [segments],
  );
}

export function DayColumn(props: DayColumnProps) {
  const placements = usePlacements(props.segments);
  const card = ({ endMinutes, event, startMinutes }: DaySegment, placement: PlacedEvent) => (
    <EventCard
      box={calculateEventBox(startMinutes, endMinutes, props.hourPx)}
      dragging={props.draggingId === event.id}
      event={event}
      key={event.id}
      look={eventLook(event, props.nowMs)}
      onOpen={props.onOpenEvent}
      placement={placement}
      // The bottom edge is where the event ends: only on the day it ends.
      resizable={
        dayOf(event.end_ms - 1, props.timezone) === props.day &&
        props.editable?.(event) === true &&
        event.ui?.saving !== true
      }
      selected={props.selectedId === event.id && event.ui?.dragging !== true}
      timezone={props.timezone}
      tone={toneOf(event, props.calendars)}
    />
  );
  return (
    <div className={styles.dayColumn} data-day={props.day} role="gridcell">
      {HOUR_LINES.map((hour) => (
        <div className={styles.hourLine} key={hour} style={{ top: hour * props.hourPx }} />
      ))}
      {props.placeholder === true &&
        PLACEHOLDER_HOURS.map(([start, length]) => (
          <span
            aria-hidden="true"
            className={styles.placeholder}
            key={start}
            style={{ height: length * props.hourPx - 3, top: start * props.hourPx }}
          />
        ))}
      {props.segments.map((segment) =>
        card(
          segment,
          placements.get(segment.event.id) ?? {
            depth: 0,
            id: segment.event.id,
            leftPercent: 0,
            widthPercent: 100,
          },
        ),
      )}
      {(props.ghosts ?? []).map((segment) =>
        card(segment, { ...GHOST_PLACE, id: segment.event.id }),
      )}
      {props.isToday && props.nowMinutes !== null && (
        <div
          aria-label={`${es.calendar.now}, ${timeLabel(props.nowMs, props.timezone)}`}
          className={styles.nowLine}
          role="img"
          style={{ top: (props.nowMinutes / 60) * props.hourPx }}
        />
      )}
    </div>
  );
}
