import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import { AgendaList } from "./AgendaList";
import { CalendarFeedback } from "./CalendarFeedback";
import { CalendarToolbar } from "./CalendarToolbar";
import { allDaySpan } from "./edit/geometry";
import { useCalendarEditing } from "./edit/useCalendarEditing";
import { WriteNotices } from "./edit/WriteNotices";
import { CalendarSide } from "./CalendarSide";
import { type CalendarEvent, visibleEvents } from "./eventLook";
import type { CalendarKeyAction } from "./keyboard";
import { hourHeight } from "./layout";
import { MonthView, usePickedDay } from "./MonthView";
import { DEFAULT_PREFERENCES } from "./preferences";
import { TimeGrid } from "./TimeGrid";
import { type CalendarRangeControls, useCalendarRange } from "./useCalendarRange";
import { useCalendarKeys } from "./useCalendarKeys";
import type { useCalendarView } from "./useCalendarView";
import type { useCalendarVisibility } from "./useCalendarVisibility";
import styles from "./CalendarView.module.css";

/** Above the weeks of the month: the header, the weekdays and the margins. */
const MONTH_CHROME_PX = 112;

export interface CalendarBodyProps {
  calendar: AppSnapshot["calendar"];
  error: string | null;
  library?: NoteLibrarySnapshot | null;
  send: (command: AppCommand) => Promise<unknown>;
  timezone: string;
  view: ReturnType<typeof useCalendarView>;
  visibility: ReturnType<typeof useCalendarVisibility>;
}

function applyKey(action: CalendarKeyAction, controls: CalendarRangeControls) {
  switch (action.kind) {
    case "view":
      return controls.choose(action.choice);
    case "step":
      return controls.step(action.direction);
    case "today":
      return controls.today();
    case "zoom":
      return controls.zoomBy(action.delta);
    case "zoomReset":
      return controls.update({ zoom: DEFAULT_PREFERENCES.zoom });
  }
}

/** The calendar once the permission and the zone are known. */
export function CalendarBody(props: CalendarBodyProps) {
  const { calendar, view, visibility } = props;
  const editing = useCalendarEditing(calendar, props.send, props.error, props.timezone, view);
  const controls = useCalendarRange(calendar.range, editing.send, props.timezone);
  useCalendarKeys((action) => applyKey(action, controls));
  const [pickedDay, pickDay] = usePickedDay(calendar.range, view.nowMs, props.timezone);
  const events = editing.display(
    visibleEvents(calendar.events, {
      hiddenCalendars: visibility.hidden,
      showDeclined: controls.preferences.showDeclined,
    }),
  );
  // While the read of the range on screen has not arrived, what is shown is
  // pending, not the period's events.
  const loading = view.snapshotPending || calendar.stale || calendar.reading.state === "unknown";
  return (
    <section className={styles.view} ref={controls.attach}>
      <CalendarToolbar
        calendar={calendar}
        controls={controls}
        hiddenCalendars={visibility.hidden}
        loading={loading && editing.readError === null}
        nowMs={view.nowMs}
        onShowAll={visibility.showAll}
        onToggleCalendar={visibility.toggle}
        timezone={props.timezone}
      />
      <CalendarFeedback
        error={editing.readError}
        onRetry={() => {
          editing.markRead();
          void view.refresh({ command: "calendar_refresh" });
        }}
      />
      <WriteNotices writes={editing.writes} />
      <div className={styles.body}>
        <div aria-busy={loading} className={styles.period}>
          <CalendarPeriod
            {...props}
            controls={controls}
            editing={editing}
            events={events}
            loading={loading}
            onPickDay={pickDay}
            pickedDay={pickedDay}
          />
        </div>
        <CalendarSide
          {...props}
          controls={controls}
          editing={editing}
          events={events}
          loading={loading}
          pickedDay={pickedDay}
        />
      </div>
    </section>
  );
}

function CalendarPeriod(
  props: CalendarBodyProps & {
    controls: CalendarRangeControls;
    editing: ReturnType<typeof useCalendarEditing>;
    events: CalendarEvent[];
    loading: boolean;
    onPickDay: (day: number) => void;
    pickedDay: number;
  },
) {
  const { calendar, controls, editing, view } = props;
  const { preferences } = controls;
  const shared = {
    calendars: calendar.calendars,
    events: props.events,
    loading: props.loading,
    nowMs: view.nowMs,
    onOpenEvent: editing.openEvent,
    range: calendar.range,
    timezone: props.timezone,
  };
  switch (calendar.range.view) {
    case "month":
      return (
        <MonthView
          {...shared}
          height={Math.max(0, controls.size.height - MONTH_CHROME_PX)}
          onDraftDay={(day) =>
            editing.editor.startDraft(allDaySpan(day, day, props.timezone), true)
          }
          onPickDay={props.onPickDay}
          pickedDay={props.pickedDay}
          showWeekends={preferences.showWeekends}
        />
      );
    case "agenda":
      return <AgendaList {...shared} onOpenDay={controls.openDay} stale={calendar.stale} />;
    case "day":
    case "days":
    case "week":
      return (
        <TimeGrid
          {...shared}
          allDayOpen={preferences.allDayOpen}
          edit={editing.grid}
          hourPx={hourHeight(controls.size.height, preferences.zoom)}
          laneEdit={editing.lane}
          onToggleAllDay={() => controls.update({ allDayOpen: !preferences.allDayOpen })}
          selectedId={editing.editor.editing?.event?.id ?? null}
          showWeekends={preferences.showWeekends}
        />
      );
  }
}
