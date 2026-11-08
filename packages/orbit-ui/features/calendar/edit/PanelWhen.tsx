import { useCopy } from "../../../i18n/CopyProvider";
import type { CalendarFrequency } from "../../../bridge/generated/CalendarFrequency";
import { Switch } from "../../../design/Switch";

import { DAY_MS } from "../date";
import { type EventForm, type Repeat, withAllDay, withStart } from "./form";
import { allDaySpan, atWall, coveredDays } from "./geometry";
import { dateValue, dayOfValue, minutesOfValue, timeValue } from "./inputs";
import { TimeField } from "./TimeField";
import styles from "./EventPanel.module.css";

interface WhenProps {
  form: EventForm;
  onChange: (form: EventForm) => void;
  timezone: string;
}

/** When: dates and hours (a new start keeps the length), the all-day switch and
 *  the repetition. */
export function PanelWhen({ form, onChange, timezone }: WhenProps) {
  const es = useCopy();
  return (
    <fieldset className={styles.when}>
      <legend className={styles.label}>{es.calendar.edit.when}</legend>
      {form.allDay ? (
        <AllDayDates form={form} onChange={onChange} timezone={timezone} />
      ) : (
        <TimedDates form={form} onChange={onChange} timezone={timezone} />
      )}
      <div className={styles.row}>
        <span className={styles.toggle}>
          <Switch
            checked={form.allDay}
            label={es.calendar.allDay}
            onCheckedChange={(allDay) => onChange(withAllDay(form, allDay, timezone))}
          />
          <span aria-hidden="true">{es.calendar.allDay}</span>
        </span>
        <RepeatField onChange={(repeat) => onChange({ ...form, repeat })} repeat={form.repeat} />
      </div>
    </fieldset>
  );
}

function TimedDates({ form, onChange, timezone }: WhenProps) {
  const es = useCopy();
  const startDate = dateValue(form.startMs, timezone);
  const endDate = dateValue(form.endMs, timezone);
  const setStart = (start: number | null) => {
    if (start !== null) onChange(withStart(form, start));
  };
  const setEnd = (end: number | null, nextDayIfBefore: boolean) => {
    if (end === null) return;
    // An end hour before the start is the next day's.
    const next = end < form.startMs && nextDayIfBefore ? end + DAY_MS : end;
    if (next >= form.startMs) onChange({ ...form, endMs: next });
  };
  const at = (date: string, minutes: number) => {
    const day = dayOfValue(date);
    return day === null ? null : atWall(day, minutes, timezone);
  };
  return (
    <div className={styles.dates}>
      <input
        aria-label={es.calendar.edit.startDate}
        onChange={(event) =>
          setStart(at(event.target.value, minutesOfValue(timeValue(form.startMs, timezone)) ?? 0))
        }
        type="date"
        value={startDate}
      />
      <TimeField
        label={es.calendar.edit.startTime}
        onCommit={(minutes) => setStart(at(startDate, minutes))}
        value={timeValue(form.startMs, timezone)}
      />
      <span aria-hidden="true">–</span>
      <TimeField
        label={es.calendar.edit.endTime}
        onCommit={(minutes) => setEnd(at(endDate, minutes), true)}
        value={timeValue(form.endMs, timezone)}
      />
      <input
        aria-label={es.calendar.edit.endDate}
        onChange={(event) =>
          setEnd(
            at(event.target.value, minutesOfValue(timeValue(form.endMs, timezone)) ?? 0),
            false,
          )
        }
        type="date"
        value={endDate}
      />
    </div>
  );
}

function AllDayDates({ form, onChange, timezone }: WhenProps) {
  const es = useCopy();
  const { first, last } = coveredDays(form, timezone);
  const value = (day: number) => dateValue(day * DAY_MS, "UTC");
  const set = (nextFirst: number, nextLast: number) =>
    onChange({ ...form, ...allDaySpan(nextFirst, nextLast, timezone) });
  return (
    <div className={styles.dates}>
      <input
        aria-label={es.calendar.edit.startDate}
        onChange={(event) => {
          const day = dayOfValue(event.target.value);
          if (day !== null) set(day, day + (last - first));
        }}
        type="date"
        value={value(first)}
      />
      <span aria-hidden="true">–</span>
      <input
        aria-label={es.calendar.edit.endDate}
        onChange={(event) => {
          const day = dayOfValue(event.target.value);
          if (day !== null && day >= first) set(first, day);
        }}
        type="date"
        value={value(last)}
      />
    </div>
  );
}

const FREQUENCIES: CalendarFrequency[] = ["daily", "weekly", "monthly", "yearly"];

function RepeatField({ onChange, repeat }: { onChange: (repeat: Repeat) => void; repeat: Repeat }) {
  const es = useCopy();
  const choice = repeat.kind === "rule" ? repeat.frequency : repeat.kind;
  const options = es.calendar.edit.repeatOptions;
  return (
    <span className={styles.repeat}>
      <select
        aria-label={es.calendar.edit.repeat}
        onChange={(event) => {
          const value = event.target.value;
          if (value === "kept") return;
          const frequency = FREQUENCIES.find((item) => item === value);
          const keep = repeat.kind === "none" ? undefined : repeat.rule;
          onChange(
            frequency
              ? {
                  frequency,
                  interval: repeat.kind === "rule" ? repeat.interval : 1,
                  kind: "rule",
                  rule: keep,
                }
              : { kind: "none" },
          );
        }}
        value={choice}
      >
        <option value="none">{options.none}</option>
        {FREQUENCIES.map((frequency) => (
          <option key={frequency} value={frequency}>
            {options[frequency]}
          </option>
        ))}
        {repeat.kind === "kept" && <option value="kept">{options.kept}</option>}
      </select>
      {repeat.kind === "rule" && <IntervalField onChange={onChange} repeat={repeat} />}
    </span>
  );
}

function IntervalField({
  onChange,
  repeat,
}: {
  onChange: (repeat: Repeat) => void;
  repeat: Extract<Repeat, { kind: "rule" }>;
}) {
  const es = useCopy();
  return (
    <label className={styles.interval}>
      <span>{es.calendar.edit.every}</span>
      <input
        aria-label={es.calendar.edit.interval}
        max={99}
        min={1}
        onChange={(event) => {
          const interval = Math.round(Number(event.target.value));
          if (interval >= 1 && interval <= 99) onChange({ ...repeat, interval });
        }}
        type="number"
        value={repeat.interval}
      />
      <span>{es.calendar.edit.units[repeat.frequency](repeat.interval)}</span>
    </label>
  );
}
