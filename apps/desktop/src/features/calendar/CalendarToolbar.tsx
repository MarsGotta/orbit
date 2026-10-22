import type { AppView as AppSnapshot } from "../../state/app-view";
import type { CalendarViewKind } from "../../bridge/generated/CalendarViewKind";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Segmented } from "../../design/Segmented";
import { Loading } from "../../design/Spinner";
import { es } from "../../i18n/es";
import { CalendarFilter } from "./CalendarFilter";
import { dayOf, dayRange, longDay, monthTitle, weekNumber } from "./date";
import { includes, type Range, type ViewChoice } from "./range";
import type { CalendarRangeControls } from "./useCalendarRange";
import { ViewOptions } from "./ViewOptions";
import styles from "./CalendarView.module.css";

type Calendar = AppSnapshot["calendar"];

interface CalendarToolbarProps {
  calendar: Calendar;
  controls: CalendarRangeControls;
  hiddenCalendars: ReadonlySet<string>;
  /** The period on screen is still being read. */
  loading: boolean;
  nowMs: number;
  onShowAll: () => void;
  onToggleCalendar: (id: string) => void;
  timezone: string;
}

/**
 * The period's name: «Octubre 2026 · semana 43», «Miércoles, 21 de
 * octubre», «21 – 23 de octubre», «Octubre 2026» or «Agenda».
 */
export function periodTitle(range: Range, timezone: string): string {
  const first = dayOf(range.start_ms, timezone);
  switch (range.view) {
    case "day":
      return longDay(first);
    case "days":
      return dayRange(first, first + range.days - 1);
    case "week": {
      // A week across two months is named by the month of its middle.
      const middle = first + Math.floor((range.days - 1) / 2);
      return `${monthTitle(middle)} · ${es.calendar.weekNumber(weekNumber(middle))}`;
    }
    case "month":
      return monthTitle(dayOf(range.anchor_ms, timezone));
    case "agenda":
      return es.calendar.agenda;
  }
}

function stepLabels(range: Range): [string, string] {
  switch (range.view) {
    case "day":
      return [es.calendar.previous.day, es.calendar.next.day];
    case "days":
      return [es.calendar.previous.days(range.days), es.calendar.next.days(range.days)];
    case "month":
      return [es.calendar.previous.month, es.calendar.next.month];
    case "week":
    case "agenda":
      return [es.calendar.previousWeek, es.calendar.nextWeek];
  }
}

/** Día · 3 días · Semana · Mes · Agenda; «3 días» names the N days on screen. */
function viewOptions(range: Range): { value: CalendarViewKind; label: string }[] {
  return [
    { label: es.calendar.day, value: "day" },
    { label: es.calendar.days(range.view === "days" ? range.days : 3), value: "days" },
    { label: es.calendar.week, value: "week" },
    { label: es.calendar.month, value: "month" },
    { label: es.calendar.agenda, value: "agenda" },
  ];
}

function choiceOf(view: CalendarViewKind, range: Range): ViewChoice {
  if (view === "day") return { days: 1, view };
  if (view === "days") return { days: range.view === "days" ? range.days : 3, view };
  return { days: 7, view };
}

/** The period, ‹ ›, «Hoy» when today is out of sight, the view and the calendars. */
export function CalendarToolbar(props: CalendarToolbarProps) {
  const range = props.calendar.range;
  const [previous, next] = stepLabels(range);
  return (
    <header className={styles.header}>
      <h2 className={styles.title}>{periodTitle(range, props.timezone)}</h2>
      {range.view === "agenda" && (
        <span className={styles.subtitle}>{es.calendar.agendaSubtitle}</span>
      )}
      <div className={styles.arrows}>
        <Button
          aria-label={previous}
          onClick={() => props.controls.step(-1)}
          size={28}
          variant="icon"
        >
          <Icon name="previous" size={16} />
        </Button>
        <Button aria-label={next} onClick={() => props.controls.step(1)} size={28} variant="icon">
          <Icon name="next" size={16} />
        </Button>
        {!includes(range, props.nowMs) && (
          <Button onClick={props.controls.today} size={28} variant="ghost">
            {es.calendar.today}
          </Button>
        )}
      </div>
      {props.loading && (
        <span className={styles.reading}>
          <Loading size={12}>{es.calendar.stale}</Loading>
        </span>
      )}
      <div className={styles.spacer} />
      <Segmented
        label={es.calendar.viewMenu}
        onChange={(view) => props.controls.choose(choiceOf(view, range))}
        options={viewOptions(range)}
        value={range.view}
      />
      <ViewOptions controls={props.controls} />
      <CalendarFilter
        calendars={props.calendar.calendars}
        hidden={props.hiddenCalendars}
        onShowAll={props.onShowAll}
        onToggle={props.onToggleCalendar}
      />
    </header>
  );
}
