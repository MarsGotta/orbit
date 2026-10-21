import { useEffect, useRef, useState } from "react";
import { Kbd } from "../../design/Kbd";
import { es } from "../../i18n/es";
import { MAX_ZOOM, MIN_ZOOM } from "./preferences";
import { MAX_GRID_DAYS, type Range, type ViewChoice } from "./range";
import type { CalendarRangeControls } from "./useCalendarRange";
import styles from "./CalendarView.module.css";

const CHOICES: { choice: ViewChoice; key: string }[] = [
  { choice: { days: 1, view: "day" }, key: "D" },
  { choice: { days: 3, view: "days" }, key: "3" },
  { choice: { days: 7, view: "week" }, key: "S" },
  { choice: { days: MAX_GRID_DAYS, view: "days" }, key: "0" },
  { choice: { days: 7, view: "month" }, key: "M" },
  { choice: { days: 7, view: "agenda" }, key: "A" },
];

export function viewName(choice: ViewChoice): string {
  switch (choice.view) {
    case "day":
      return es.calendar.day;
    case "days":
      return es.calendar.days(choice.days);
    case "week":
      return es.calendar.week;
    case "month":
      return es.calendar.month;
    case "agenda":
      return es.calendar.agenda;
  }
}

/** The view selector (P9-20): the views, their keys and the local options. */
export function ViewMenu({ controls, range }: { controls: CalendarRangeControls; range: Range }) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (menuRef.current !== null && !menuRef.current.contains(event.target as Node))
        setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);
  const current = viewName(range);
  return (
    <div
      className={styles.calendarSelector}
      onKeyDown={(event) => {
        if (event.key === "Escape") setOpen(false);
      }}
      ref={menuRef}
    >
      <button
        aria-expanded={open}
        aria-label={`${es.calendar.viewMenu}: ${current}`}
        className={styles.viewButton}
        onClick={() => setOpen((value) => !value)}
        type="button"
      >
        {current}
        <span aria-hidden="true" className={styles.caret} />
      </button>
      {open && <ViewChoices controls={controls} onChosen={() => setOpen(false)} range={range} />}
    </div>
  );
}

function ViewChoices(props: {
  controls: CalendarRangeControls;
  onChosen: () => void;
  range: Range;
}) {
  const { controls, range } = props;
  const chosen = (choice: ViewChoice) =>
    choice.view === range.view && (choice.view !== "days" || choice.days === range.days);
  return (
    <div aria-label={es.calendar.viewMenu} className={styles.calendarChoices} role="group">
      <div className={styles.viewList}>
        {CHOICES.map(({ choice, key }) => (
          <button
            aria-pressed={chosen(choice)}
            className={styles.viewOption}
            key={`${choice.view}-${choice.days}`}
            onClick={() => {
              controls.choose(choice);
              props.onChosen();
            }}
            type="button"
          >
            <span>{viewName(choice)}</span>
            <Kbd>{key}</Kbd>
          </button>
        ))}
      </div>
      <ViewOptions controls={controls} />
      <p className={styles.viewKeys}>{es.calendar.viewKeys}</p>
    </div>
  );
}

function ViewOptions({ controls }: { controls: CalendarRangeControls }) {
  const { preferences, update } = controls;
  return (
    <div className={styles.viewOptions}>
      <label className={styles.calendarChoice}>
        <span>{es.calendar.showWeekends}</span>
        <input
          checked={preferences.showWeekends}
          onChange={() => update({ showWeekends: !preferences.showWeekends })}
          type="checkbox"
        />
      </label>
      <label className={styles.calendarChoice}>
        <span>{es.calendar.showDeclined}</span>
        <input
          checked={preferences.showDeclined}
          onChange={() => update({ showDeclined: !preferences.showDeclined })}
          type="checkbox"
        />
      </label>
      <fieldset className={styles.weekStart}>
        <legend>{es.calendar.weekStartsOn}</legend>
        {(["monday", "sunday"] as const).map((day) => (
          <label key={day}>
            <input
              checked={preferences.weekStart === day}
              name="calendar-week-start"
              onChange={() => update({ weekStart: day })}
              type="radio"
            />
            {day === "monday" ? es.calendar.monday : es.calendar.sunday}
          </label>
        ))}
      </fieldset>
      <div className={styles.zoom}>
        <button
          aria-label={es.calendar.zoomOut}
          disabled={preferences.zoom <= MIN_ZOOM}
          onClick={() => controls.zoomBy(-0.1)}
          type="button"
        >
          −
        </button>
        <span>{es.calendar.zoomValue(Math.round(preferences.zoom * 100))}</span>
        <button
          aria-label={es.calendar.zoomIn}
          disabled={preferences.zoom >= MAX_ZOOM}
          onClick={() => controls.zoomBy(0.1)}
          type="button"
        >
          +
        </button>
      </div>
    </div>
  );
}
