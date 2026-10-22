import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { ViewError } from "../../design/ViewError";
import { macPermissions } from "../../i18n/es/permissions";
import { es } from "../../i18n/es";
import { CalendarBody } from "./CalendarBody";
import { useCalendarView } from "./useCalendarView";
import { useCalendarVisibility } from "./useCalendarVisibility";
import styles from "./CalendarView.module.css";

interface CalendarViewProps {
  error: string | null;
  send: (command: AppCommand) => Promise<unknown>;
  snapshot: AppSnapshot;
}

/// The macOS permission is the only decision. With it the calendar shows
/// every calendar on the Mac; without it the view says how to grant it; while
/// the Mac has not said yet, the view says nothing — never «denied».
export default function CalendarView({ error, send, snapshot }: CalendarViewProps) {
  const authorization = snapshot.calendar.authorization;
  if (snapshot.calendar.reading.blocker === "privacy_stopped") {
    return (
      <EmptyState
        action={es.calendar.rearm}
        description={es.calendar.privacyStoppedDescription}
        error={error}
        onAction={() => void send({ command: "rearm" })}
        title={es.calendar.privacyStoppedTitle}
      />
    );
  }
  if (snapshot.calendar.reading.blocker === "helper_not_running") {
    return (
      <EmptyState
        description={es.calendar.helperNotRunningDescription}
        error={error}
        title={es.calendar.helperNotRunningTitle}
      />
    );
  }
  if (authorization === "denied") {
    return (
      <EmptyState
        action={es.calendar.openSystemSettings}
        description={es.calendar.deniedDescription}
        error={error}
        onAction={() =>
          void send({ command: "open_system_privacy", pane: macPermissions.calendar.pane })
        }
        title={es.calendar.deniedTitle}
      />
    );
  }
  if (authorization === "not_determined") {
    return (
      <EmptyState
        action={es.calendar.connect}
        description={es.calendar.connectDescription}
        error={error}
        onAction={() =>
          void send({ command: "calendar", request: { action: "request_permission" } })
        }
        title={es.calendar.connectTitle}
      />
    );
  }
  if (authorization !== "granted") return <Quiet error={error} />;

  return <CalendarContent error={error} send={send} snapshot={snapshot} />;
}

function CalendarContent({ error, send, snapshot }: CalendarViewProps) {
  const calendar = snapshot.calendar;
  const view = useCalendarView(calendar, snapshot.system_timezone ?? "", send);
  const visibility = useCalendarVisibility(calendar.calendars);
  if (view.timezone === null) {
    // Without the Mac's zone no hour can be placed. A zone still on its way
    // says nothing; one the system could not give is a failure.
    const failed = (snapshot.system_timezone_error ?? "") !== "";
    return failed ? (
      <EmptyState
        description={es.calendar.timezoneFailed}
        error={error}
        title={es.calendar.unavailableTitle}
      />
    ) : (
      <Quiet error={error} />
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

/** Nothing known yet: an unknown is not a failure and is not painted. */
function Quiet({ error }: { error: string | null }) {
  return (
    <section className={styles.empty}>
      <ViewError error={error} />
    </section>
  );
}

function EmptyState({
  action,
  description,
  error,
  onAction,
  title,
}: {
  action?: string;
  description: string;
  error: string | null;
  onAction?: () => void;
  title: string;
}) {
  return (
    <section className={styles.empty}>
      <ViewError error={error} />
      <div>
        <h1>{title}</h1>
        <p>{description}</p>
        {Boolean(action) && onAction && (
          <Button onClick={onAction} size={36} variant="accent">
            {action}
          </Button>
        )}
      </div>
    </section>
  );
}
