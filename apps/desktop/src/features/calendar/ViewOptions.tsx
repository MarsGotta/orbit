import { useState } from "react";
import { Icon } from "../../design/Icon";
import { Popover } from "../../design/Popover";
import { es } from "../../i18n/es";
import { MAX_ZOOM, MIN_ZOOM } from "./preferences";
import { MAX_GRID_DAYS } from "./range";
import type { CalendarRangeControls } from "./useCalendarRange";
import styles from "./CalendarView.module.css";

/**
 * What the five views do not say: weekends, declined events, the day the
 * week starts, the zoom, fourteen days and the keys. This Mac only.
 */
export function ViewOptions({ controls }: { controls: CalendarRangeControls }) {
  const [open, setOpen] = useState(false);
  const { preferences, update } = controls;
  return (
    <Popover
      align="end"
      label={es.calendar.viewOptions}
      onOpenChange={setOpen}
      open={open}
      trigger={<Icon name="settings" size={16} title={es.calendar.viewOptions} />}
      triggerClassName={styles.iconTrigger}
    >
      <div className={styles.menu}>
        <label className={styles.menuRow}>
          <span>{es.calendar.showWeekends}</span>
          <input
            checked={preferences.showWeekends}
            onChange={() => update({ showWeekends: !preferences.showWeekends })}
            type="checkbox"
          />
        </label>
        <label className={styles.menuRow}>
          <span>{es.calendar.showDeclined}</span>
          <input
            checked={preferences.showDeclined}
            onChange={() => update({ showDeclined: !preferences.showDeclined })}
            type="checkbox"
          />
        </label>
        <WeekStart controls={controls} />
        <Zoom controls={controls} />
        <button
          className={styles.menuAction}
          onClick={() => {
            controls.choose({ days: MAX_GRID_DAYS, view: "days" });
            setOpen(false);
          }}
          type="button"
        >
          {es.calendar.fourteenDays}
        </button>
        <p className={styles.menuHint}>{es.calendar.viewKeys}</p>
      </div>
    </Popover>
  );
}

function WeekStart({ controls }: { controls: CalendarRangeControls }) {
  const { preferences, update } = controls;
  return (
    <fieldset className={styles.menuGroup}>
      <legend>{es.calendar.weekStartsOn}</legend>
      {(["monday", "sunday"] as const).map((day) => (
        <label className={styles.menuRow} key={day}>
          <span>{day === "monday" ? es.calendar.monday : es.calendar.sunday}</span>
          <input
            checked={preferences.weekStart === day}
            name="calendar-week-start"
            onChange={() => update({ weekStart: day })}
            type="radio"
          />
        </label>
      ))}
    </fieldset>
  );
}

function Zoom({ controls }: { controls: CalendarRangeControls }) {
  const { preferences } = controls;
  return (
    <div className={styles.zoom}>
      <button
        aria-label={es.calendar.zoomOut}
        className={styles.zoomButton}
        disabled={preferences.zoom <= MIN_ZOOM}
        onClick={() => controls.zoomBy(-0.1)}
        type="button"
      >
        <Icon name="minus" size={14} />
      </button>
      <span>{es.calendar.zoomValue(Math.round(preferences.zoom * 100))}</span>
      <button
        aria-label={es.calendar.zoomIn}
        className={styles.zoomButton}
        disabled={preferences.zoom >= MAX_ZOOM}
        onClick={() => controls.zoomBy(0.1)}
        type="button"
      >
        <Icon name="plus" size={14} />
      </button>
    </div>
  );
}
