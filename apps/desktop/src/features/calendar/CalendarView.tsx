import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { macPermissions } from "../../i18n/es/permissions";
import { es } from "../../i18n/es";
import { CalendarBody } from "./CalendarBody";
import { CalendarEmpty, CalendarLoading, ConnectCalendar } from "./CalendarStates";
import { useCalendarView } from "./useCalendarView";
import { useCalendarVisibility } from "./useCalendarVisibility";

interface CalendarViewProps {
  error: string | null;
  send: (command: AppCommand) => Promise<unknown>;
  snapshot: AppSnapshot;
}

/// The macOS permission is the only decision. With it the calendar shows
/// every calendar on the Mac; without it the view says how to grant it; while
/// the Mac has not said yet, the view shows the calendar loading — never
/// «denied», and never a blank view.
export default function CalendarView({ error, send, snapshot }: CalendarViewProps) {
  const authorization = snapshot.calendar.authorization;
  if (snapshot.calendar.reading.blocker === "privacy_stopped") {
    return (
      <CalendarEmpty
        action={
          <Button onClick={() => void send({ command: "rearm" })} size={36} variant="accent">
            {es.calendar.rearm}
          </Button>
        }
        error={error}
        icon="stopAll"
        title={es.calendar.privacyStoppedTitle}
      >
        {es.calendar.privacyStoppedDescription}
      </CalendarEmpty>
    );
  }
  if (snapshot.calendar.reading.blocker === "helper_not_running") {
    return (
      <CalendarEmpty error={error} icon="warning" title={es.calendar.helperNotRunningTitle}>
        {es.calendar.helperNotRunningDescription}
      </CalendarEmpty>
    );
  }
  if (authorization === "denied") {
    return (
      <CalendarEmpty
        action={
          <Button
            onClick={() =>
              void send({ command: "open_system_privacy", pane: macPermissions.calendar.pane })
            }
            size={36}
            variant="accent"
          >
            {es.calendar.openSystemSettings}
          </Button>
        }
        error={error}
        icon="locked"
        title={es.calendar.deniedTitle}
      >
        {es.calendar.deniedDescription}
      </CalendarEmpty>
    );
  }
  if (authorization === "not_determined") {
    return (
      <ConnectCalendar
        error={error}
        onConnect={() => send({ command: "calendar", request: { action: "request_permission" } })}
      />
    );
  }
  if (authorization !== "granted") return <CalendarLoading error={error} />;

  return <CalendarContent error={error} send={send} snapshot={snapshot} />;
}

function CalendarContent({ error, send, snapshot }: CalendarViewProps) {
  const calendar = snapshot.calendar;
  const view = useCalendarView(calendar, snapshot.system_timezone ?? "", send);
  const visibility = useCalendarVisibility(calendar.calendars);
  if (view.timezone === null) {
    // Without the Mac's zone no hour can be placed. A zone still on its way
    // is loading; one the system could not give is a failure.
    const failed = (snapshot.system_timezone_error ?? "") !== "";
    return failed ? (
      <CalendarEmpty error={error} icon="warning" title={es.calendar.unavailableTitle}>
        {es.calendar.timezoneFailed}
      </CalendarEmpty>
    ) : (
      <CalendarLoading error={error} />
    );
  }
  return (
    <CalendarBody
      calendar={calendar}
      error={error}
      library={snapshot.notes?.library}
      send={send}
      timezone={view.timezone}
      view={view}
      visibility={visibility}
    />
  );
}
