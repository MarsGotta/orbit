import type { AppView as AppSnapshot } from "../../state/app-view";
import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { CalendarSpan } from "../../bridge/generated/CalendarSpan";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { es } from "../../i18n/es";
import type { EventForm } from "./edit/form";
import { lockedReason } from "./edit/notices";
import { PanelActions } from "./edit/PanelActions";
import { PanelFields } from "./edit/PanelFields";
import { PanelWhen } from "./edit/PanelWhen";
import type { Editing } from "./edit/useEventEditor";
import { EventPeople, EventRecord } from "./EventMeeting";
import { type CalendarEvent, calendarTone, toneStyle } from "./eventLook";
import { repeatOfForm, repeatText, whenText } from "./eventText";
import { EventTemplate } from "./series/EventTemplate";
import styles from "./edit/EventPanel.module.css";

type Calendars = AppSnapshot["calendar"]["calendars"];

export interface EventDetailsProps {
  calendars: Calendars;
  editing: Editing;
  /** Templates and their assignment to a series. */
  library?: NoteLibrarySnapshot | null;
  nowMs: number;
  onChange: (form: EventForm) => void;
  onClose: () => void;
  onDelete: (span: CalendarSpan) => void;
  /** «Unirse y grabar»: opens the call and records the event. */
  onJoin: (event: CalendarEvent, url: string) => void;
  /** «Solo grabar». */
  onRecord: (event: CalendarEvent) => void;
  onSave: (span: CalendarSpan) => void;
  send: (command: AppCommand) => Promise<unknown> | void;
  timezone: string;
}

/**
 * The event's panel, 360 px beside the calendar, in the design's order:
 * calendar, title, when and repetition, «Unirse y grabar» and «Solo grabar»,
 * place, notes, reminders, availability, the user's answer, the guests and the
 * note's template. Changes accumulate until they are saved; an event the user
 * cannot change shows the lock and why, and no fields.
 */
export function EventDetails(props: EventDetailsProps) {
  const { editing } = props;
  const event = editing.event;
  const locked = event === null ? null : lockedReason(event, props.calendars);
  const calendar = props.calendars.find((item) => item.id === editing.form.calendarId);
  const saving = editing.saving?.state === "pending";
  return (
    <aside aria-label={es.calendar.eventDetails} className={styles.panel} data-event-panel="true">
      <PanelHead calendar={calendar} onClose={props.onClose} />
      <div className={styles.body}>
        {event !== null && locked !== null ? (
          <LockedHeading event={event} locked={locked} timezone={props.timezone} />
        ) : (
          <fieldset className={styles.group} disabled={saving}>
            <EditableHeading {...props} />
          </fieldset>
        )}
        {event !== null && event.end_ms > props.nowMs && (
          <EventRecord event={event} onJoin={props.onJoin} onRecord={props.onRecord} />
        )}
        {locked === null && (
          <fieldset className={styles.group} disabled={saving}>
            <PanelFields
              alarmsNotEditable={event?.alarms_not_editable}
              calendars={props.calendars}
              draft={event === null}
              form={editing.form}
              notesLeftOutBytes={event?.notes_left_out_bytes}
              onChange={props.onChange}
            />
          </fieldset>
        )}
        {locked !== null && Boolean(event?.location) && (
          <p className={styles.line}>{event?.location}</p>
        )}
        {event !== null && <EventPeople event={event} locked={locked} />}
        {event?.calendar_item_id != null && (
          <EventTemplate
            calendarItemId={event.calendar_item_id}
            library={props.library}
            send={props.send}
          />
        )}
      </div>
      {locked === null && (
        <PanelActions editing={editing} onDelete={props.onDelete} onSave={props.onSave} />
      )}
    </aside>
  );
}

function PanelHead(props: { calendar: Calendars[number] | undefined; onClose: () => void }) {
  return (
    <header className={styles.head}>
      {props.calendar !== undefined && (
        <span className={styles.calendarName}>
          <span
            aria-hidden="true"
            className={styles.dot}
            style={toneStyle(calendarTone(props.calendar.color))}
          />
          {props.calendar.title}
        </span>
      )}
      <Button
        aria-label={es.calendar.closeDetails}
        className={styles.close}
        onClick={props.onClose}
        size={28}
        variant="icon"
      >
        <Icon name="close" size={16} />
      </Button>
    </header>
  );
}

function EditableHeading(props: EventDetailsProps) {
  const { editing } = props;
  const draft = editing.event === null;
  // A draft opens with its dates and hours; an event shows them in a line
  // that opens them.
  const [open, setOpen] = useState(draft);
  const repeat = repeatOfForm(editing.form.repeat);
  return (
    <div className={styles.heading}>
      <input
        aria-label={es.calendar.edit.title}
        autoFocus={draft}
        className={styles.title}
        onChange={(change) => props.onChange({ ...editing.form, title: change.target.value })}
        placeholder={es.calendar.edit.newEvent}
        value={editing.form.title}
      />
      <button
        aria-expanded={open}
        className={styles.whenButton}
        onClick={() => setOpen((value) => !value)}
        title={es.calendar.edit.changeWhen}
        type="button"
      >
        {whenText(editing.form, props.timezone)}
        <Icon name="chevron" size={14} />
      </button>
      {repeat !== null && <p className={styles.repeatLine}>{repeat}</p>}
      {open && (
        <PanelWhen form={editing.form} onChange={props.onChange} timezone={props.timezone} />
      )}
    </div>
  );
}

function LockedHeading({
  event,
  locked,
  timezone,
}: {
  event: CalendarEvent;
  locked: NonNullable<ReturnType<typeof lockedReason>>;
  timezone: string;
}) {
  const reason =
    locked === "organizer"
      ? es.calendar.edit.readOnly.organizer(event.organizer?.name ?? null)
      : es.calendar.edit.readOnly[locked]();
  return (
    <div className={styles.heading}>
      <h2 className={styles.title}>{event.title}</h2>
      <p className={styles.whenText}>
        {whenText(
          { allDay: event.all_day === true, endMs: event.end_ms, startMs: event.start_ms },
          timezone,
        )}
      </p>
      {event.recurrence !== undefined && (
        <p className={styles.repeatLine}>{repeatText(event.recurrence)}</p>
      )}
      <p className={styles.locked}>
        <Icon name="locked" size={14} />
        <span>{reason}</span>
      </p>
    </div>
  );
}
