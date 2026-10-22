import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { CalendarSpan } from "../../bridge/generated/CalendarSpan";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { timeLabel } from "./date";
import type { EventForm } from "./edit/form";
import { lockedReason } from "./edit/notices";
import { PanelActions } from "./edit/PanelActions";
import { PanelFields } from "./edit/PanelFields";
import { PanelWhen } from "./edit/PanelWhen";
import type { Editing } from "./edit/useEventEditor";
import { EventMeeting } from "./EventMeeting";
import { EventTemplate } from "./series/EventTemplate";
import type { CalendarEvent } from "./eventLook";
import panel from "./edit/EventPanel.module.css";
import styles from "./CalendarView.module.css";

export interface EventDetailsProps {
  calendars: AppSnapshot["calendar"]["calendars"];
  editing: Editing;
  /** Templates and their assignment to a series. */
  library?: NoteLibrarySnapshot | null;
  onChange: (form: EventForm) => void;
  onClose: () => void;
  onDelete: (span: CalendarSpan) => void;
  onJoin: (url: string) => void;
  onRecord: () => void;
  onSave: (span: CalendarSpan) => void;
  send: (command: AppCommand) => Promise<unknown> | void;
  timezone: string;
}

/**
 * The event's panel. The user edits title, when, repetition, place,
 * notes, calendar, reminders and availability; the changes accumulate until
 * they save. An event they cannot change reads as before, with the cause in
 * one line. The read-only detail's answer, guests, «Unirse» and «Grabar» stay.
 */
export function EventDetails(props: EventDetailsProps) {
  const { editing } = props;
  const event = editing.event;
  const locked = event === null ? null : lockedReason(event, props.calendars);
  return (
    <aside
      aria-label={es.calendar.eventDetails}
      className={[styles.eventDetails, panel.panel].join(" ")}
      data-event-panel="true"
    >
      <div className={styles.dialogHeader}>
        {event !== null && locked !== null ? (
          <ReadOnlyHeading event={event} timezone={props.timezone} />
        ) : (
          <input
            aria-label={es.calendar.edit.title}
            autoFocus={event === null}
            className={panel.title}
            onChange={(change) => props.onChange({ ...editing.form, title: change.target.value })}
            placeholder={es.calendar.edit.newEvent}
            value={editing.form.title}
          />
        )}
        <Button aria-label={es.calendar.close} onClick={props.onClose} size={28} variant="ghost">
          ×
        </Button>
      </div>
      {locked !== null && <p className={panel.locked}>{es.calendar.edit.readOnly[locked]}</p>}
      {locked === null && <Editable {...props} />}
      {locked !== null && Boolean(event?.location) && (
        <p>
          <strong>{es.calendar.location}:</strong> {event?.location}
        </p>
      )}
      {event !== null && (
        <EventMeeting event={event} onJoin={props.onJoin} onRecord={props.onRecord} />
      )}
      {event?.calendar_item_id != null && (
        <EventTemplate
          calendarItemId={event.calendar_item_id}
          library={props.library}
          send={props.send}
        />
      )}
      {locked === null && (
        <PanelActions editing={editing} onDelete={props.onDelete} onSave={props.onSave} />
      )}
    </aside>
  );
}

function Editable(props: EventDetailsProps) {
  const { editing } = props;
  return (
    <>
      <PanelWhen form={editing.form} onChange={props.onChange} timezone={props.timezone} />
      <PanelFields
        alarmsNotEditable={editing.event?.alarms_not_editable}
        calendars={props.calendars}
        draft={editing.event === null}
        form={editing.form}
        notesLeftOutBytes={editing.event?.notes_left_out_bytes}
        onChange={props.onChange}
      />
    </>
  );
}

function ReadOnlyHeading({ event, timezone }: { event: CalendarEvent; timezone: string }) {
  return (
    <div>
      <h2>{event.title}</h2>
      <p>
        {event.all_day === true
          ? es.calendar.allDay
          : es.calendar.timeRange(
              timeLabel(event.start_ms, timezone),
              timeLabel(event.end_ms, timezone),
            )}
      </p>
    </div>
  );
}
