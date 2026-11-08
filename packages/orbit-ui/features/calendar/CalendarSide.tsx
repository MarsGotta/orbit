import { DayAgenda } from "./DayAgenda";
import type { CalendarBodyProps } from "./CalendarBody";
import type { useCalendarEditing } from "./edit/useCalendarEditing";
import { EventDetails } from "./EventDetails";
import type { CalendarEvent } from "./eventLook";
import type { CalendarRangeControls } from "./useCalendarRange";

/** Beside the period: the event's panel, or the month's picked day. */
export function CalendarSide(
  props: CalendarBodyProps & {
    controls: CalendarRangeControls;
    editing: ReturnType<typeof useCalendarEditing>;
    events: CalendarEvent[];
    loading: boolean;
    pickedDay: number;
  },
) {
  const { calendar, controls, editing, view } = props;
  const panel = editing.editor.editing;
  if (panel !== null)
    return (
      <EventDetails
        calendars={calendar.calendars}
        editing={panel}
        key={panel.event?.id ?? "draft"}
        library={props.library}
        nowMs={view.nowMs}
        onChange={editing.editor.setForm}
        onClose={editing.editor.close}
        onDelete={editing.editor.remove}
        onJoin={(event, url) => {
          void props.send({ command: "open_url", url });
          view.startMeeting(event);
        }}
        onRecord={(event) => view.startMeeting(event)}
        onSave={editing.editor.save}
        send={props.send}
        timezone={props.timezone}
      />
    );
  if (calendar.range.view !== "month") return null;
  return (
    <DayAgenda
      calendars={calendar.calendars}
      day={props.pickedDay}
      events={props.events}
      loading={props.loading}
      nowMs={view.nowMs}
      onOpenDay={controls.openDay}
      onOpenEvent={(event) => {
        controls.openDay(event.start_ms);
        editing.openEvent(event);
      }}
      timezone={props.timezone}
    />
  );
}
