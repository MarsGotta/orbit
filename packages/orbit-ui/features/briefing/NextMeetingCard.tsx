import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { BriefingNextSnapshot } from "../../bridge/generated/BriefingNextSnapshot";
import { Button } from "../../design/Button";

import { timeLabel } from "../calendar/date";
import { PersonalReminder } from "./PersonalReminder";
import { PreviousActions } from "./PreviousActions";
import type { Send } from "./send";
import styles from "./BriefingView.module.css";

interface Props {
  meeting: BriefingNextSnapshot;
  send: Send;
  nowMs: number;
  timezone: string;
  /** The hub can be reached (its reminder is asked again when it comes back). */
  online: boolean;
}

const hasUrl = (url: string | null): url is string => url !== null && url !== "";

/** «11:00 – 11:45 · videollamada». */
export function meetingWhen(meeting: BriefingNextSnapshot, timezone: string): string {
  const range = `${timeLabel(meeting.start_ms, timezone)} – ${timeLabel(meeting.end_ms, timezone)}`;
  return hasUrl(meeting.url) ? `${range} · ${getCopy().briefing.videoCall}` : range;
}

/**
 * The next meeting: when it starts, what was said last time and who goes,
 * with «Unirse y grabar» (or «Solo grabar» with no call to join) and the
 * earlier note. Below the line, what was left open and a personal reminder.
 */
export function NextMeetingCard({ meeting, send, nowMs, timezone, online }: Props) {
  const es = useCopy();
  const minutes = Math.max(0, Math.ceil((meeting.start_ms - nowMs) / 60_000));
  const label = meeting.start_ms <= nowMs ? es.briefing.nextNow : es.briefing.nextStartsIn(minutes);
  const record = () => {
    if (hasUrl(meeting.url)) void send({ command: "open_url", url: meeting.url });
    void send({
      command: "meeting_start",
      event_id: meeting.event_id,
      ...(meeting.event ? { event: meeting.event } : {}),
    });
  };
  const previous = meeting.previous;
  return (
    <section aria-labelledby="hoy-next" className={`${styles.card} ${styles.next}`}>
      <header className={styles.nextHeader}>
        <span aria-hidden="true" className={styles.nextDot} />
        <strong>{label}</strong>
        <span>{meetingWhen(meeting, timezone)}</span>
      </header>
      <h2 id="hoy-next">{meeting.title}</h2>
      <LastTime previous={previous} />
      <Attendees meeting={meeting} />
      <div className={styles.actions}>
        <Button onClick={record} size={40} variant="accent">
          {hasUrl(meeting.url) ? es.briefing.joinRecord : es.briefing.onlyRecord}
        </Button>
        {previous && (
          <Button
            onClick={() => {
              window.location.hash = `#/notes/${encodeURIComponent(previous.recording_id)}`;
            }}
            size={40}
          >
            {es.briefing.previousNote}
          </Button>
        )}
      </div>
      <div className={styles.split}>
        {meeting.previous_actions ? (
          <PreviousActions actions={meeting.previous_actions} send={send} />
        ) : (
          <span />
        )}
        <PersonalReminder
          calendarItemId={meeting.event?.calendar_item_id ?? null}
          hub={meeting.reminder}
          online={online}
          send={send}
          seriesKey={meeting.series_key}
        />
      </div>
    </section>
  );
}

/** One line of what the last note of this series said; nothing when there is none. */
function LastTime({ previous }: { previous: BriefingNextSnapshot["previous"] }) {
  const es = useCopy();
  const context =
    previous?.summary != null && previous.summary !== ""
      ? previous.summary
      : previous?.title != null && previous.title !== ""
        ? es.briefing.previousTitle(previous.title)
        : null;
  return context === null ? null : <p className={styles.lastTime}>{context}</p>;
}

function Attendees({ meeting }: { meeting: BriefingNextSnapshot }) {
  const es = useCopy();
  if (meeting.attendees.length === 0) return null;
  return (
    <div className={styles.people}>
      <span aria-hidden="true" className={styles.faces}>
        {meeting.attendees.slice(0, 4).map((attendee) => (
          <span key={attendee.name}>{attendee.initial}</span>
        ))}
      </span>
      {es.briefing.withPeople(meeting.attendees.map((attendee) => attendee.name))}
    </div>
  );
}
