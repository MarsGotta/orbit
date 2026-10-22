import { CalendarEvent as EventBox } from "../../design/CalendarEvent";
import { es } from "../../i18n/es";
import { timeLabel } from "./date";
import {
  type CalendarEvent,
  type CalendarTone,
  type EventLook,
  responseLabel,
  toneStyle,
} from "./eventLook";
import type { PlacedEvent } from "./layout";
import styles from "./EventCard.module.css";

interface EventCardProps {
  box: { height: number; top: number };
  /** A gesture is moving this card; it stays here, dimmed. */
  dragging?: boolean;
  event: CalendarEvent;
  look: EventLook;
  onOpen: (event: CalendarEvent) => void;
  placement: PlacedEvent;
  /** Its bottom edge can be dragged to change the end. */
  resizable?: boolean;
  /** Its panel is open: the ring of its colour. */
  selected?: boolean;
  timezone: string;
  tone: CalendarTone;
}

/** What the card says under its title: its hours, or what is happening to it. */
function subtitle(event: CalendarEvent, look: EventLook, timezone: string): string {
  if (event.ui?.saving === true) return es.calendar.write.saving;
  if (look.recording) return es.calendar.recording;
  return es.calendar.timeRange(
    timeLabel(event.start_ms, timezone),
    timeLabel(event.end_ms, timezone),
  );
}

/**
 * One event in the time grid: the design's calendar event, placed. The slot
 * around it carries the place, the identity the gestures read and the state
 * the card shows while it is drafted, dragged, saved or recorded.
 */
export function EventCard(props: EventCardProps) {
  const { box, event, look, placement } = props;
  const ghost = event.ui?.dragging === true;
  return (
    <div
      aria-busy={event.ui?.saving === true ? true : undefined}
      className={styles.slot}
      data-declined={look.declined ? "true" : undefined}
      data-draft={event.ui?.draft === true ? "true" : undefined}
      data-dragging={props.dragging === true ? "true" : undefined}
      data-event-id={ghost ? undefined : event.id}
      data-ghost={ghost ? "true" : undefined}
      data-recording={look.recording ? "true" : undefined}
      data-saving={event.ui?.saving === true ? "true" : undefined}
      inert={ghost}
      style={{
        ...toneStyle(props.tone),
        height: box.height,
        left: `${placement.leftPercent}%`,
        top: box.top,
        width: `${placement.widthPercent}%`,
        zIndex: 1 + placement.depth,
      }}
    >
      <EventBox
        color={props.tone}
        height={box.height}
        onOpen={() => props.onOpen(event)}
        past={look.past && event.ui?.draft !== true}
        response={responseLabel(event)}
        selected={props.selected === true}
        time={subtitle(event, look, props.timezone)}
        title={event.title}
      />
      {look.recording && <span aria-hidden="true" className={styles.pulse} />}
      {props.resizable === true && (
        <span
          aria-hidden="true"
          className={styles.resize}
          data-resize="end"
          title={es.calendar.edit.resizeEnd}
        />
      )}
    </div>
  );
}
