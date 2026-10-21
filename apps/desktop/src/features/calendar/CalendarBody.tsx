import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import { AgendaList } from "./AgendaList";
import { CalendarFeedback } from "./CalendarFeedback";
import { CalendarToolbar } from "./CalendarToolbar";
import { allDaySpan } from "./edit/geometry";
import { useCalendarEditing } from "./edit/useCalendarEditing";
import { WriteNotices } from "./edit/WriteNotices";
import { EventDetails } from "./EventDetails";
import { type CalendarEvent, visibleEvents } from "./eventLook";
import type { CalendarKeyAction } from "./keyboard";
import { hourHeight } from "./layout";
import { MonthView } from "./MonthView";
import { DEFAULT_PREFERENCES } from "./preferences";
import { TimeGrid } from "./TimeGrid";
import { type CalendarRangeControls, useCalendarRange } from "./useCalendarRange";
import { useCalendarKeys } from "./useCalendarKeys";
import type { useCalendarView } from "./useCalendarView";
import type { useCalendarVisibility } from "./useCalendarVisibility";
import styles from "./CalendarView.module.css";

/** Above the weeks of the month: the toolbar, the reading line, the weekdays. */
const MONTH_CHROME_PX = 120;

interface CalendarBodyProps {
  calendar: AppSnapshot["calendar"];
  error: string | null;
  library?: NoteLibrarySnapshot | null;
  send: (command: AppCommand) => Promise<void>;
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

/** The calendar once the permission and the zone are known (P9-20). */
export function CalendarBody(props: CalendarBodyProps) {
  const { calendar, view, visibility } = props;
  const editing = useCalendarEditing(calendar, props.send, props.error, props.timezone, view);
  const controls = useCalendarRange(calendar.range, editing.send, props.timezone);
  useCalendarKeys((action) => applyKey(action, controls));
  const events = editing.display(
    visibleEvents(calendar.events, {
      hiddenCalendars: visibility.hidden,
      showDeclined: controls.preferences.showDeclined,
    }),
  );
  return (
    <section className={styles.calendarView} ref={controls.attach}>
      <CalendarToolbar
        calendar={calendar}
        controls={controls}
        hiddenCalendars={visibility.hidden}
        nowMs={view.nowMs}
        onShowAll={visibility.showAll}
        onStartMeeting={() => view.startMeeting()}
        onToggleCalendar={visibility.toggle}
        timezone={props.timezone}
      />
      <CalendarFeedback
        calendar={calendar}
        error={editing.readError}
        onRetry={() => {
          editing.markRead();
          void view.refresh({ command: "calendar_refresh" });
        }}
        pending={view.snapshotPending}
      />
      <WriteNotices writes={editing.writes} />
      {/* While the read of the range on screen has not arrived, what is shown
          is pending, not the period's events (P9-20). */}
      <div aria-busy={calendar.stale} className={styles.period}>
        <CalendarPeriod {...props} controls={controls} editing={editing} events={events} />
      </div>
      {editing.editor.editing !== null && (
        <EventDetails
          calendars={calendar.calendars}
          editing={editing.editor.editing}
          library={props.library}
          onChange={editing.editor.setForm}
          onClose={editing.editor.close}
          onDelete={editing.editor.remove}
          onJoin={(url) => void props.send({ command: "open_url", url })}
          onRecord={() => view.startMeeting(editing.editor.editing?.event ?? undefined)}
          onSave={editing.editor.save}
          send={props.send}
          timezone={props.timezone}
        />
      )}
    </section>
  );
}

function CalendarPeriod(
  props: CalendarBodyProps & {
    controls: CalendarRangeControls;
    editing: ReturnType<typeof useCalendarEditing>;
    events: CalendarEvent[];
  },
) {
  const { calendar, controls, editing, view } = props;
  const { preferences } = controls;
  const shared = {
    calendars: calendar.calendars,
    events: props.events,
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
          onOpenDay={controls.openDay}
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
          showWeekends={preferences.showWeekends}
        />
      );
  }
}
