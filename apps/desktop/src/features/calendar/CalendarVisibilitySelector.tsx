import type { AppView as AppSnapshot } from "../../state/app-view";
import { useEffect, useRef, useState, type RefObject } from "react";

import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import styles from "./CalendarView.module.css";

type CalendarItem = AppSnapshot["calendar"]["calendars"][number];

interface CalendarVisibilitySelectorProps {
  calendars: CalendarItem[];
  hidden: ReadonlySet<string>;
  onShowAll: () => void;
  onToggle: (id: string) => void;
}

export function CalendarVisibilitySelector(props: CalendarVisibilitySelectorProps) {
  const [open, setOpen] = useState(false);
  const selectorRef = useRef<HTMLDivElement>(null);
  const visibleCount = props.calendars.filter((item) => !props.hidden.has(item.id)).length;
  const countLabel =
    visibleCount === props.calendars.length
      ? es.calendar.macCalendars(props.calendars.length)
      : es.calendar.visibleCalendars(visibleCount, props.calendars.length);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (selectorRef.current !== null && !selectorRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);
  // Calendars not listed yet: nothing to choose and nothing to say.
  if (props.calendars.length === 0) return null;
  return (
    <div className={styles.calendarSelector} ref={selectorRef}>
      <button
        aria-expanded={open}
        className={styles.calendarCount}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (event.key === "Escape") setOpen(false);
        }}
        type="button"
      >
        <span aria-hidden="true" className={styles.dotStack}>
          {props.calendars
            .filter((item) => !props.hidden.has(item.id))
            .slice(0, 4)
            .map((item) => (
              <span className={`${styles.calendarDot} ${styles[item.color]}`} key={item.id} />
            ))}
        </span>
        <span>{countLabel}</span>
      </button>
      {open && (
        <CalendarChoices {...props} onClose={() => setOpen(false)} selectorRef={selectorRef} />
      )}
    </div>
  );
}

function CalendarChoices({
  calendars,
  hidden,
  onClose,
  onShowAll,
  onToggle,
  selectorRef,
}: CalendarVisibilitySelectorProps & {
  onClose: () => void;
  selectorRef: RefObject<HTMLDivElement | null>;
}) {
  return (
    <div
      aria-label={es.calendar.visibleCalendarsLabel}
      className={styles.calendarChoices}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          onClose();
          selectorRef.current?.querySelector("button")?.focus();
        }
      }}
      role="group"
    >
      <div className={styles.choicesHeader}>
        <strong>{es.calendar.visibleCalendarsLabel}</strong>
        <Button onClick={onShowAll} size={28} variant="ghost">
          {es.calendar.showAllCalendars}
        </Button>
      </div>
      <div className={styles.choicesList}>
        {calendars.map((item) => (
          <label className={styles.calendarChoice} key={item.id}>
            <span
              aria-hidden="true"
              className={`${styles.calendarDot} ${styles[item.color] ?? ""}`}
            />
            <span>{item.title}</span>
            <input
              checked={!hidden.has(item.id)}
              onChange={() => onToggle(item.id)}
              type="checkbox"
            />
          </label>
        ))}
      </div>
    </div>
  );
}
