import type { AppCommand } from "../bridge/generated";
import type { MeetingSnapshot } from "../bridge/generated/MeetingSnapshot";
import { Button } from "../design/Button";
import { Spinner } from "../design/Spinner";
import { useSpanish } from "../i18n/AssistantCopy";
import { effectiveTimezone } from "../local-time";
import { settingsRoute } from "../state/ui";
import { mmss } from "../time-format";
import { useNow } from "../use-now";
import { endingLabel } from "./call-countdown";
import { DiscardButton, DiscardQuestion, useDiscardQuestion } from "./DiscardRecording";
import { AskMenu } from "./meeting-question/AskMenu";
import { voiceStatusText } from "./meeting-question/copy";
import { useRecordedTime } from "./recorded-time";
import styles from "./SidebarCard.module.css";

/** «Preguntar a Deimos» reads its answer: `false` when the daemon refused it. */
type Send = (command: AppCommand) => unknown;
type Copy = ReturnType<typeof useSpanish>;

/** What it records and until when: «Audio del sistema · hasta las 12:00». */
function audioLine(meeting: MeetingSnapshot, timezone: string, es: Copy): string {
  const panel = meeting.call_panel;
  const audio =
    meeting.audio === "application" && panel && panel.state !== "offer"
      ? es.recordingCard.audioOf(panel.application)
      : es.recordingCard.audio[meeting.audio];
  if (meeting.ends_at_ms === null) return audio;
  const zone = effectiveTimezone(timezone) ?? undefined;
  const time = new Intl.DateTimeFormat("es-ES", {
    hour: "numeric",
    minute: "2-digit",
    timeZone: zone,
  }).format(meeting.ends_at_ms);
  return `${audio} · ${es.recordingCard.until(time)}`;
}

/** Whether «Oye Deimos» listens, and why not; nothing while it gets ready. */
function voiceLine(meeting: MeetingSnapshot, es: Copy): string | null {
  if (meeting.phase === "paused") return es.recordingCard.notInPause;
  const status = meeting.voice_trigger;
  if (status?.state === "listening") return es.recordingCard.listening;
  return voiceStatusText(status, es);
}

function silentLine(meeting: MeetingSnapshot, es: Copy): string | null {
  const silent = meeting.tracks.find((track) => track.level === "silent");
  if (silent?.name === "remote_app") return es.meeting.systemSilent;
  if (silent?.name === "mic_local") return es.meeting.microphoneSilent;
  return null;
}

function stateLabel(meeting: MeetingSnapshot, es: Copy): string {
  if (meeting.phase === "starting") return es.recordingCard.starting;
  if (meeting.phase === "finishing") return es.recordingCard.finishing;
  if (meeting.phase === "paused") return es.recordingCard.paused;
  return es.recordingCard.recording;
}

function Controls({ meeting, send }: { meeting: MeetingSnapshot; send: Send }) {
  const es = useSpanish();
  const discard = useDiscardQuestion(meeting);
  if (meeting.phase === "finishing") return null;
  if (discard.asking && meeting.recording_id !== null) {
    return <DiscardQuestion onKeep={discard.keep} recordingId={meeting.recording_id} send={send} />;
  }
  return (
    <>
      <AskMenu meeting={meeting} send={send} />
      <div className={styles.controls}>
        {meeting.phase === "recording" && (
          <Button onClick={() => send({ command: "meeting_pause" })} size={32}>
            {es.recordingCard.pause}
          </Button>
        )}
        {meeting.phase === "paused" && (
          <Button onClick={() => send({ command: "meeting_resume" })} size={32}>
            {es.recordingCard.resume}
          </Button>
        )}
        <Button onClick={() => send({ command: "meeting_finish" })} size={32} variant="dangerSoft">
          {es.recordingCard.finish}
        </Button>
      </div>
      {discard.offered && <DiscardButton onAsk={discard.ask} />}
    </>
  );
}

/**
 * The recording in progress, at the top of the sidebar in every view, with
 * its controls always in sight (the pill hid them behind a hover).
 */
export function RecordingCard({
  meeting,
  send,
  timezone,
}: {
  meeting: MeetingSnapshot;
  send: Send;
  timezone: string;
}) {
  const es = useSpanish();
  const active = meeting.phase !== "idle";
  const now = useNow(active ? 1_000 : null);
  const recorded = useRecordedTime(meeting, now);
  if (!active) return null;
  const busy = meeting.phase === "starting" || meeting.phase === "finishing";
  const voice = voiceLine(meeting, es);
  const silent = silentLine(meeting, es);
  const ending = meeting.call_panel?.state === "ending" ? meeting.call_panel : null;
  return (
    <section aria-label={es.recordingCard.label} className={styles.card}>
      <div className={styles.head}>
        {busy ? (
          <Spinner size={12} />
        ) : (
          <span aria-hidden="true" className={styles.dot} data-phase={meeting.phase} />
        )}
        <span aria-live="polite" className={styles.state} data-phase={meeting.phase}>
          {stateLabel(meeting, es)}
        </span>
        {meeting.started_at_ms !== null && (
          <span aria-label={es.recordingCard.elapsed} className={styles.clock} role="timer">
            {mmss(recorded)}
          </span>
        )}
      </div>
      <div className={styles.title}>{meeting.title ?? es.meeting.defaultTitle}</div>
      <div className={styles.meta}>{audioLine(meeting, timezone, es)}</div>
      {voice !== null && <div className={styles.meta}>{voice}</div>}
      {silent !== null && (
        <div className={styles.warn}>
          <span>{silent}</span> · <a href={settingsRoute("privacidad")}>{es.recordingCard.check}</a>
        </div>
      )}
      {ending && (
        <div className={styles.ending}>
          <span>{endingLabel(ending.ends_at_ms, now)}</span>
          <Button
            onClick={() => send({ command: "recording_keep", recording_id: ending.recording_id })}
            size={28}
          >
            {es.callDetection.keep}
          </Button>
        </div>
      )}
      <Controls meeting={meeting} send={send} />
    </section>
  );
}
