import { useCopy } from "../../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useState } from "react";
import { Popover } from "../../design/Popover";

import { calendarTone, toneStyle } from "./eventLook";
import styles from "./CalendarView.module.css";

type CalendarItem = AppSnapshot["calendar"]["calendars"][number];

interface CalendarFilterProps {
  calendars: CalendarItem[];
  hidden: ReadonlySet<string>;
  onShowAll: () => void;
  onToggle: (id: string) => void;
}

/**
 * «4 calendarios» with their dots: hides or shows each calendar in this view
 * only. Deimos and the rest of Orbit go on reading every calendar.
 */
export function CalendarFilter(props: CalendarFilterProps) {
  const es = useCopy();
  const [open, setOpen] = useState(false);
  // Calendars not listed yet: nothing to choose and nothing to say.
  if (props.calendars.length === 0) return null;
  const visible = props.calendars.filter((item) => !props.hidden.has(item.id));
  const label =
    visible.length === props.calendars.length
      ? es.calendar.macCalendars(props.calendars.length)
      : es.calendar.visibleCalendars(visible.length, props.calendars.length);
  return (
    <Popover
      align="end"
      label={es.calendar.visibleCalendarsLabel}
      onOpenChange={setOpen}
      open={open}
      trigger={
        <>
          <span aria-hidden="true" className={styles.dots}>
            {visible.slice(0, 4).map((item) => (
              <span
                className={styles.dot}
                key={item.id}
                style={toneStyle(calendarTone(item.color))}
              />
            ))}
          </span>
          {label}
        </>
      }
      triggerClassName={styles.filterTrigger}
    >
      <div className={styles.menu}>
        <div className={styles.menuHeader}>
          <strong>{es.calendar.visibleCalendarsLabel}</strong>
          <button className={styles.menuLink} onClick={props.onShowAll} type="button">
            {es.calendar.showAllCalendars}
          </button>
        </div>
        {props.calendars.map((item) => (
          <label className={styles.menuRow} key={item.id}>
            <span
              aria-hidden="true"
              className={styles.dot}
              style={toneStyle(calendarTone(item.color))}
            />
            <span className={styles.menuLabel}>{item.title}</span>
            <input
              checked={!props.hidden.has(item.id)}
              onChange={() => props.onToggle(item.id)}
              type="checkbox"
            />
          </label>
        ))}
      </div>
    </Popover>
  );
}
