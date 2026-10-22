import type { AppCommand } from "../../bridge/generated";
import type { BriefingNextSnapshot } from "../../bridge/generated/BriefingNextSnapshot";
import type { BriefingSourceState } from "../../bridge/generated/BriefingSourceState";
import { Avatar, type AvatarColor } from "../../design/Avatar";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { MeetingTopics } from "./MeetingTopics";
import styles from "./BriefingView.module.css";

function avatarColor(color: string): AvatarColor {
  return ["accent", "orange", "blue", "green", "gray"].includes(color)
    ? (color as AvatarColor)
    : "accent";
}

function clockLabel(atMs: number, timezone: string): string {
  return new Intl.DateTimeFormat("es-ES", {
    hour: "2-digit",
    minute: "2-digit",
    timeZone: timezone,
  }).format(atMs);
}

interface Props {
  meeting: BriefingNextSnapshot;
  send: (command: AppCommand) => void;
  nowMs: number;
  timezone: string;
  recordingState?: BriefingSourceState;
}

export function NextMeetingCard({ meeting, send, nowMs, timezone, recordingState }: Props) {
  const recording =
    recordingState?.state === "unknown" || recordingState?.state === "absent"
      ? false
      : meeting.recording === "recording";
  const minutes = Math.max(0, Math.ceil((meeting.start_ms - nowMs) / 60_000));
  const regionLabel =
    meeting.start_ms <= nowMs
      ? es.briefing.inProgress
      : `${es.briefing.startsIn} ${minutes} ${es.briefing.minutes}`;
  const label = recording ? es.briefing.recording : regionLabel;
  const record = () => {
    if (meeting.url != null && meeting.url !== "") send({ command: "open_url", url: meeting.url });
    send({
      command: "meeting_start",
      event_id: meeting.event_id,
      ...(meeting.event ? { event: meeting.event } : {}),
    });
  };
  return (
    <section aria-label={regionLabel} className={styles.nextCard}>
      <header className={styles.nextHeader}>
        <span className={styles.liveDot} />
        <strong>{label}</strong>
        <span>{clockLabel(meeting.start_ms, timezone)}</span>
      </header>
      <div className={styles.nextBody}>
        <MeetingDetails
          meeting={meeting}
          onRecord={record}
          recording={recording}
          send={send}
          timezone={timezone}
        />
        <MeetingTopics meeting={meeting} />
      </div>
    </section>
  );
}

function MeetingDetails({
  meeting,
  onRecord,
  recording,
  send,
  timezone,
}: {
  meeting: BriefingNextSnapshot;
  onRecord: () => void;
  recording: boolean;
  send: Props["send"];
  timezone: string;
}) {
  const previous = meeting.previous;
  return (
    <div className={styles.meetingInfo}>
      <div className={styles.meetingTitle}>
        <span />
        <strong>{es.briefing.nextPrefix}</strong>
        <h2>{meeting.title}</h2>
        <time>{`${clockLabel(meeting.start_ms, timezone)} – ${clockLabel(meeting.end_ms, timezone)}`}</time>
      </div>
      <PreviousContext previous={previous} />
      <div className={styles.attendees}>
        {meeting.attendees.map((attendee) => (
          <span className={styles.attendee} key={attendee.name}>
            <Avatar
              color={avatarColor(attendee.color)}
              initial={attendee.initial}
              label={attendee.name}
              size={18}
            />
            {attendee.name}
          </span>
        ))}
      </div>
      <div className={styles.actions}>
        <Button
          onClick={() => (recording ? send({ command: "meeting_finish" }) : onRecord())}
          size={36}
          variant="accent"
        >
          {recording ? es.briefing.finish : es.briefing.joinRecord}
        </Button>
        {previous && (
          <Button
            onClick={() => {
              window.location.hash = `#/notes/${previous.recording_id}`;
            }}
            size={36}
          >
            {es.briefing.previousNote}
          </Button>
        )}
      </div>
    </div>
  );
}

/** What the last note of this series said; nothing when there is none. */
function PreviousContext({ previous }: { previous: BriefingNextSnapshot["previous"] }) {
  const context =
    previous?.summary != null && previous.summary !== ""
      ? previous.summary
      : previous?.title != null && previous.title !== ""
        ? es.briefing.previousTitle(previous.title)
        : null;
  return context === null ? null : <p>{context}</p>;
}
