import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { Button } from "../../design/Button";
import { TextLink } from "../../design/TextLink";

import type { lockedReason } from "./edit/notices";
import { type CalendarEvent, placePreview } from "./eventLook";
import styles from "./edit/EventPanel.module.css";

/**
 * «Unirse y grabar» for a known video call (it opens the call and records the
 * event), «Solo grabar», and how long the recording lasts. Recording starts
 * at once: there is nothing to confirm.
 */
export function EventRecord(props: {
  event: CalendarEvent;
  onJoin: (event: CalendarEvent, url: string) => void;
  onRecord: (event: CalendarEvent) => void;
}) {
  const es = useCopy();
  const { event } = props;
  const conference = event.conference_url ?? null;
  const link = conference !== null ? placePreview({ ...event, location: null }) : null;
  return (
    <div className={styles.record}>
      <div className={styles.recordButtons}>
        {conference !== null && (
          <Button
            className={styles.join}
            onClick={() => props.onJoin(event, conference)}
            size={36}
            variant="accent"
          >
            {es.calendar.joinRecord}
          </Button>
        )}
        <Button onClick={() => props.onRecord(event)} size={36}>
          <span aria-hidden="true" className={styles.recDot} />
          {es.calendar.recordOnly}
        </Button>
      </div>
      <p className={styles.caption}>
        {link !== null
          ? `${link} · ${es.calendar.recordsUntil}`
          : es.calendar.recordsUntil.charAt(0).toUpperCase() + es.calendar.recordsUntil.slice(1)}
      </p>
      {event.url != null && event.url !== "" && conference === null && (
        <TextLink href={event.url} rel="noreferrer" target="_blank">
          {es.calendar.openLink}
        </TextLink>
      )}
    </div>
  );
}

type Tone = "ok" | "warn" | "muted" | "plain";

/** The user's answer, as the design says it; `null` when there is nothing to say. */
function answerOf(
  event: CalendarEvent,
  locked: ReturnType<typeof lockedReason>,
): { text: string; tone: Tone } | null {
  switch (event.self_response ?? "none") {
    case "accepted":
      return { text: getCopy().calendar.selfResponse.accepted, tone: "ok" };
    case "tentative":
      return { text: getCopy().calendar.selfResponse.tentative, tone: "warn" };
    case "needs_action":
      return { text: getCopy().calendar.unansweredHere, tone: "warn" };
    case "declined":
      return { text: getCopy().calendar.selfResponse.declined, tone: "muted" };
    case "none":
      break;
  }
  // No invitation to the user: theirs when they organize it, or when it has
  // no guests in a calendar they can write to.
  const ownByGuests = (event.attendees?.length ?? 0) === 0 && locked !== "read_only";
  return event.organizer?.is_self === true || ownByGuests
    ? { text: getCopy().calendar.yours, tone: "plain" }
    : null;
}

type Attendee = NonNullable<CalendarEvent["attendees"]>[number];

function isOrganizer(attendee: Attendee, event: CalendarEvent): boolean {
  const organizer = event.organizer;
  if (organizer == null) return false;
  if (organizer.email != null && attendee.email != null) return organizer.email === attendee.email;
  return organizer.name === attendee.name;
}

function guestStatus(attendee: Attendee, event: CalendarEvent): { text: string; tone: Tone } {
  if (isOrganizer(attendee, event)) return { text: getCopy().calendar.organizes, tone: "plain" };
  switch (attendee.response ?? "none") {
    case "accepted":
      return { text: getCopy().calendar.guestResponse.accepted, tone: "ok" };
    case "tentative":
      return { text: getCopy().calendar.guestResponse.tentative, tone: "warn" };
    case "declined":
      return { text: getCopy().calendar.guestResponse.declined, tone: "muted" };
    case "needs_action":
      return { text: getCopy().calendar.guestResponse.needs_action, tone: "muted" };
    case "none":
      return { text: "", tone: "muted" };
  }
}

/** «Tu respuesta» and «Invitados · N», each guest with their answer in colour. */
export function EventPeople(props: {
  event: CalendarEvent;
  locked: ReturnType<typeof lockedReason>;
}) {
  const es = useCopy();
  const { event } = props;
  const answer = answerOf(event, props.locked);
  const attendees = event.attendees ?? [];
  if (answer === null && attendees.length === 0) return null;
  return (
    <div className={styles.people}>
      <hr className={styles.rule} />
      {answer !== null && (
        <p className={styles.answer}>
          <span className={styles.fieldLabel}>{es.calendar.yourAnswer}</span>
          <span className={styles.answerValue} data-tone={answer.tone}>
            {answer.text}
          </span>
        </p>
      )}
      {attendees.length > 0 && (
        <div>
          <p className={styles.fieldLabel}>{es.calendar.guestsTitle(attendees.length)}</p>
          <ul className={styles.guests}>
            {attendees.map((attendee) => {
              const status = guestStatus(attendee, event);
              return (
                <li key={`${attendee.name}-${attendee.email ?? ""}`}>
                  <span aria-hidden="true" className={styles.initial}>
                    {attendee.name.slice(0, 1).toUpperCase()}
                  </span>
                  <span className={styles.guestName}>
                    {attendee.is_self === true ? es.calendar.you(attendee.name) : attendee.name}
                  </span>
                  <span className={styles.guestStatus} data-tone={status.tone}>
                    {status.text}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
