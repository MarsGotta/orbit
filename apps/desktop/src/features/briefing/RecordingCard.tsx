import type { AppView as AppSnapshot } from "../../state/app-view";
import type { BriefingNextSnapshot } from "../../bridge/generated/BriefingNextSnapshot";
import type { VoiceStatusSnapshot } from "../../bridge/generated/VoiceStatusSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { mmss } from "../../time-format";
import { useNow } from "../../use-now";
import { timeLabel } from "../calendar/date";
import { meetingWhen } from "./NextMeetingCard";
import { PreviousActions } from "./PreviousActions";
import type { Send } from "./send";
import styles from "./BriefingView.module.css";

interface Props {
  meeting: AppSnapshot["meeting"];
  /** The day's next meeting when it is the one being recorded. */
  event: BriefingNextSnapshot | null;
  send: Send;
  timezone: string;
}

/** «Grabando · 12:34», «En pausa», or what the recording is doing now. */
function phaseLabel(meeting: AppSnapshot["meeting"], nowMs: number): string {
  switch (meeting.phase) {
    case "recording":
      return es.briefing.recordingClock(mmss(nowMs - (meeting.started_at_ms ?? nowMs)));
    case "paused":
      return es.briefing.paused;
    case "starting":
      return es.meeting.starting;
    case "finishing":
    case "idle":
      return es.meeting.finishing;
  }
}

/**
 * Hoy while a meeting records (Grabando.dc.html): the time, private notes
 * and «Terminar», what was left open last time and the shortcuts to ask
 * Deimos during the meeting.
 */
export function RecordingCard({ meeting, event, send, timezone }: Props) {
  const nowMs = useNow(meeting.phase === "recording" ? 1_000 : null);
  const recordingId = meeting.recording_id;
  const when =
    event !== null
      ? meetingWhen(event, timezone)
      : meeting.ends_at_ms !== null
        ? es.briefing.recordsUntil(timeLabel(meeting.ends_at_ms, timezone))
        : null;
  return (
    <section aria-labelledby="hoy-recording" className={`${styles.card} ${styles.next}`}>
      <header className={styles.nextHeader} data-phase={meeting.phase}>
        <span aria-hidden="true" className={styles.recDot} />
        <strong>{phaseLabel(meeting, nowMs)}</strong>
        {when !== null && <span>{when}</span>}
      </header>
      <h2 id="hoy-recording">{meeting.title ?? es.meeting.defaultTitle}</h2>
      <p className={styles.lastTime}>{es.briefing.recordingHint}</p>
      <div className={styles.actions}>
        {recordingId !== null && (
          <Button
            onClick={() => {
              window.location.hash = `#/notes/${encodeURIComponent(recordingId)}`;
            }}
            size={40}
            variant="accent"
          >
            {es.briefing.privateNotes}
          </Button>
        )}
        {meeting.phase !== "finishing" && (
          <Button onClick={() => void send({ command: "meeting_finish" })} size={40}>
            {es.briefing.finish}
          </Button>
        )}
      </div>
      <div className={styles.split}>
        {event?.previous_actions ? (
          <PreviousActions actions={event.previous_actions} send={send} />
        ) : (
          <span />
        )}
        <DuringMeeting listening={listens(meeting.voice_trigger)} />
      </div>
    </section>
  );
}

/** «Oye Deimos» listens, also right after a phrase it heard was left without a question. */
function listens(status: VoiceStatusSnapshot | undefined): boolean {
  return status?.state === "listening" || status?.state === "abandoned";
}

/** «Durante la reunión»: how to ask Deimos and how to finish from any app. */
function DuringMeeting({ listening }: { listening: boolean }) {
  const rows: Array<[string, string]> = [
    ...(listening ? [[es.briefing.askAloud, es.briefing.heyDeimos] as [string, string]] : []),
    [es.briefing.askSilently, es.briefing.pttShortcut],
    [es.briefing.finishAnywhere, es.briefing.meetingShortcut],
  ];
  return (
    <div className={styles.during}>
      <h3>{es.briefing.during}</h3>
      <dl>
        {rows.map(([what, how]) => (
          <div key={what}>
            <dt>{what}</dt>
            <dd>{how}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
