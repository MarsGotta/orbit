import { Button } from "../../design/Button";
import { TextLink } from "../../design/TextLink";
import { es } from "../../i18n/es";
import { VideoGlyph } from "./AgendaList";
import type { CalendarEvent } from "./eventLook";
import styles from "./CalendarView.module.css";

/**
 * The meeting side of an event (P9-20): how Mars answered, who is invited
 * and how each answered, «Unirse» for a known video call, and «Grabar».
 */
export function EventMeeting(props: {
  event: CalendarEvent;
  onJoin: (url: string) => void;
  onRecord: () => void;
}) {
  const answer = props.event.self_response;
  return (
    <>
      {answer != null && answer !== "none" && (
        <p className={styles.selfResponse} data-response={answer}>
          {es.calendar.selfResponse[answer]}
        </p>
      )}
      <Attendees event={props.event} />
      <Actions {...props} />
    </>
  );
}

function Actions(props: {
  event: CalendarEvent;
  onJoin: (url: string) => void;
  onRecord: () => void;
}) {
  const { event } = props;
  const conference = event.conference_url;
  return (
    <div className={styles.dialogActions}>
      {event.url != null && event.url !== "" && conference == null && (
        <TextLink href={event.url} rel="noreferrer" target="_blank">
          {es.calendar.openLink}
        </TextLink>
      )}
      {conference != null && (
        <Button onClick={() => props.onJoin(conference)}>
          <VideoGlyph />
          {es.calendar.join}
        </Button>
      )}
      <Button onClick={props.onRecord} variant="accent">
        {es.calendar.record}
      </Button>
    </div>
  );
}

function Attendees({ event }: { event: CalendarEvent }) {
  const attendees = event.attendees ?? [];
  if (attendees.length === 0) return null;
  return (
    <div>
      <strong>{es.calendar.attendees}</strong>
      <ul className={styles.attendees}>
        {attendees.map((attendee) => {
          const answer = attendee.response ?? "none";
          return (
            <li data-response={answer} key={`${attendee.name}-${attendee.email ?? ""}`}>
              <span>{attendee.name}</span>
              {answer !== "none" && <span>{es.calendar.guestResponse[answer]}</span>}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
