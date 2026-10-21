import type { AppView as AppSnapshot } from "../../../state/app-view";
import { TextInput } from "../../../design/TextInput";
import { es } from "../../../i18n/es";
import palette from "../CalendarPalette.module.css";
import type { EventForm } from "./form";
import styles from "./EventPanel.module.css";

type Calendars = AppSnapshot["calendar"]["calendars"];

interface FieldsProps {
  calendars: Calendars;
  /** Notes longer than Orbit carries are edited in Calendar, not here. */
  notesLeftOutBytes?: number;
  /** Reminders Orbit does not edit, as the Mac counted them. */
  alarmsNotEditable?: number;
  draft: boolean;
  form: EventForm;
  onChange: (form: EventForm) => void;
}

/** Place, notes, calendar, reminders and availability (P9-22). */
export function PanelFields(props: FieldsProps) {
  const { form, onChange } = props;
  return (
    <div className={styles.fields}>
      <TextInput
        aria-label={es.calendar.location}
        className={styles.text}
        onChange={(event) => onChange({ ...form, location: event.target.value })}
        placeholder={es.calendar.edit.locationPlaceholder}
        value={form.location}
      />
      {props.notesLeftOutBytes !== undefined ? (
        <p className={styles.hint}>{es.calendar.edit.notesTooLong}</p>
      ) : (
        <textarea
          aria-label={es.calendar.edit.notes}
          className={styles.notes}
          onChange={(event) => onChange({ ...form, notes: event.target.value })}
          placeholder={es.calendar.edit.notesPlaceholder}
          rows={2}
          value={form.notes}
        />
      )}
      <CalendarField calendars={props.calendars} form={form} onChange={onChange} />
      <Reminders
        alarmsNotEditable={props.alarmsNotEditable}
        draft={props.draft}
        form={form}
        onChange={onChange}
      />
      <Availability form={form} onChange={onChange} />
    </div>
  );
}

/** Only the calendars that admit events, each with its color. */
function CalendarField({ calendars, form, onChange }: Omit<FieldsProps, "draft">) {
  const writable = calendars.filter(
    (calendar) => calendar.can_edit !== false || calendar.id === form.calendarId,
  );
  if (writable.length === 0) return null;
  const current = calendars.find((calendar) => calendar.id === form.calendarId);
  return (
    <label className={styles.row}>
      <span
        aria-hidden="true"
        className={[styles.swatch, palette[current?.color ?? "accent"] ?? palette.accent].join(" ")}
      />
      <select
        aria-label={es.calendar.edit.calendar}
        className={styles.select}
        onChange={(event) => onChange({ ...form, calendarId: event.target.value })}
        value={form.calendarId ?? ""}
      >
        {form.calendarId === null && <option value="">{es.calendar.edit.calendar}</option>}
        {writable.map((calendar) => (
          <option key={calendar.id} value={calendar.id}>
            {calendar.title}
          </option>
        ))}
      </select>
    </label>
  );
}

/** The usual reminders, and any other the event already has. */
function reminderChoices(read: readonly number[]): [number, string][] {
  const usual = es.calendar.edit.reminderOptions.map(([minutes, label]): [number, string] => [
    minutes,
    label,
  ]);
  const other = read
    .filter((minutes) => !usual.some(([value]) => value === minutes))
    .map((minutes): [number, string] => [minutes, es.calendar.edit.minutesBefore(minutes)]);
  return [...usual, ...other].sort((left, right) => left[0] - right[0]);
}

function ReminderHint({
  form,
  alarmsNotEditable = 0,
}: Omit<FieldsProps, "calendars" | "onChange">) {
  if (form.alarms === null && alarmsNotEditable > 0)
    return <p className={styles.hint}>{es.calendar.edit.remindersTooMany}</p>;
  if (alarmsNotEditable > 0)
    return <p className={styles.hint}>{es.calendar.edit.remindersKept(alarmsNotEditable)}</p>;
  return null;
}

function Reminders(props: Omit<FieldsProps, "calendars">) {
  const { form, onChange } = props;
  const chosen = form.alarms ?? [];
  // More than Orbit edits: none is touched from here (a write would drop some).
  const locked = form.alarms === null && (props.alarmsNotEditable ?? 0) > 0;
  const toggle = (minutes: number) =>
    onChange({
      ...form,
      alarms: chosen.includes(minutes)
        ? chosen.filter((item) => item !== minutes)
        : [...chosen, minutes],
    });
  return (
    <div aria-label={es.calendar.edit.reminders} className={styles.chips} role="group">
      <span className={styles.label}>{es.calendar.edit.reminders}</span>
      {reminderChoices(chosen).map(([minutes, label]) => (
        <button
          aria-pressed={chosen.includes(minutes)}
          className={styles.chip}
          disabled={locked}
          key={minutes}
          onClick={() => toggle(minutes)}
          type="button"
        >
          {label}
        </button>
      ))}
      <ReminderHint {...props} />
    </div>
  );
}

/** Busy or free; a calendar that keeps no availability shows nothing. */
function Availability({ form, onChange }: Omit<FieldsProps, "calendars" | "draft">) {
  if (form.availability === null || form.availability === "not_supported") return null;
  return (
    <div aria-label={es.calendar.edit.availability} className={styles.chips} role="group">
      <span className={styles.label}>{es.calendar.edit.availability}</span>
      {(["busy", "free"] as const).map((value) => (
        <button
          aria-pressed={form.availability === value}
          className={styles.chip}
          key={value}
          onClick={() => onChange({ ...form, availability: value })}
          type="button"
        >
          {es.calendar.edit[value]}
        </button>
      ))}
    </div>
  );
}
