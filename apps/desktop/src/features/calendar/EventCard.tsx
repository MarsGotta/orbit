import { es } from "../../i18n/es";
import { timeLabel } from "./date";
import { type CalendarEvent, type EventLook, placePreview } from "./eventLook";
import type { PlacedEvent } from "./layout";
import palette from "./CalendarPalette.module.css";
import styles from "./EventCard.module.css";

/** Below this height a card has room for the title and the time only. */
const PLACE_LINE_MIN_PX = 52;

interface EventCardProps {
  box: { height: number; top: number };
  /** A gesture is moving this card; it stays here, dimmed. */
  dragging?: boolean;
  event: CalendarEvent;
  look: EventLook;
  onOpen: (event: CalendarEvent) => void;
  paletteName: string;
  placement: PlacedEvent;
  /** Its bottom edge can be dragged to change the end. */
  resizable?: boolean;
  timezone: string;
}

function subtitle(event: CalendarEvent, look: EventLook, timezone: string): string {
  if (event.ui?.saving === true) return es.calendar.write.saving;
  if (look.recording) return es.calendar.recording;
  if (event.ui?.dragging === true)
    return es.calendar.timeRange(
      timeLabel(event.start_ms, timezone),
      timeLabel(event.end_ms, timezone),
    );
  return timeLabel(event.start_ms, timezone);
}

/** One event in the time grid: a card styled with Orbit's tokens. */
export function EventCard(props: EventCardProps) {
  const { box, event, look, placement } = props;
  const muted = look.done || look.declined;
  const place = box.height >= PLACE_LINE_MIN_PX ? placePreview(event) : null;
  const ghost = event.ui?.dragging === true;
  return (
    <button
      aria-busy={event.ui?.saving === true ? true : undefined}
      aria-hidden={ghost ? true : undefined}
      className={[
        styles.event,
        palette[props.paletteName] ?? palette.accent,
        look.striped ? styles.striped : "",
        look.recording ? styles.recording : "",
        box.height < 34 ? styles.compact : "",
      ].join(" ")}
      data-declined={look.declined ? "true" : undefined}
      data-done={look.done ? "true" : undefined}
      data-draft={event.ui?.draft === true ? "true" : undefined}
      data-dragging={props.dragging === true ? "true" : undefined}
      data-event-id={ghost ? undefined : event.id}
      data-ghost={ghost ? "true" : undefined}
      data-recording={look.recording ? "true" : undefined}
      data-response={event.self_response ?? undefined}
      data-saving={event.ui?.saving === true ? "true" : undefined}
      data-striped={look.striped ? "true" : undefined}
      onClick={() => props.onOpen(event)}
      style={{
        height: box.height,
        left: `${placement.leftPercent}%`,
        opacity: muted ? 0.5 : undefined,
        textDecorationLine: muted ? "line-through" : undefined,
        top: box.top,
        width: `${placement.widthPercent}%`,
        zIndex: 1 + placement.depth,
      }}
      tabIndex={ghost ? -1 : undefined}
      type="button"
    >
      <span className={styles.copy}>
        <strong>{event.title}</strong>
        <time>{subtitle(event, look, props.timezone)}</time>
        {place !== null && <span className={styles.place}>{place}</span>}
      </span>
      <EventIcon look={look} />
      {look.recording && <span aria-hidden="true" className={styles.pulse} />}
      {props.resizable === true && (
        <span
          aria-hidden="true"
          className={styles.resize}
          data-resize="end"
          title={es.calendar.edit.resizeEnd}
        />
      )}
    </button>
  );
}

export function EventIcon({ look }: { look: Pick<EventLook, "icon"> }) {
  if (look.icon === "unanswered")
    return <span aria-label={es.calendar.unanswered} className={styles.unanswered} role="img" />;
  return null;
}
