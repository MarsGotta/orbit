import type { AppView as AppSnapshot } from "../../state/app-view";
import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { TextLink } from "../../design/TextLink";
import { useNow } from "../../use-now";
import { mmss } from "../../time-format";
import { es } from "../../i18n/es";
import { endingLabel } from "../../app/call-countdown";
import { PillAsk, PillAskButton, PillVoiceStatus } from "../../app/meeting-question/PillAsk";
import styles from "./Meetings.module.css";

function elapsedLabel(startedAt: number | null, now: number): string {
  return mmss(Math.max(0, now - (startedAt ?? now)));
}

function silentLabel(meeting: AppSnapshot["meeting"]): string | null {
  const silent = meeting.tracks.find((track) => track.level === "silent");
  if (silent?.name === "remote_app") return es.meeting.systemSilent;
  if (silent?.name === "mic_local") return es.meeting.microphoneSilent;
  return null;
}

function statusLabel(meeting: AppSnapshot["meeting"], now: number): string {
  const title = meeting.title ?? es.meeting.defaultTitle;
  const elapsed = elapsedLabel(meeting.started_at_ms, now);
  if (meeting.phase === "starting") return `${es.meeting.starting} · ${title}`;
  if (meeting.phase === "paused") return `${es.meeting.paused} · ${title} · ${elapsed}`;
  if (meeting.phase === "finishing") return es.meeting.finishing;
  return `${es.meeting.recording} · ${title} · ${elapsed}`;
}

export interface MeetingPillProps {
  send: (command: AppCommand) => void;
  snapshot: AppSnapshot;
}

function ActiveMeetingPill({
  label,
  meeting,
  send,
  silent,
}: {
  label: string;
  meeting: AppSnapshot["meeting"];
  send: MeetingPillProps["send"];
  silent: string | null;
}) {
  const ending = meeting.call_panel?.state === "ending" ? meeting.call_panel : null;
  const [asking, setAsking] = useState(false);
  return (
    <div
      aria-label={label}
      className={`${styles.meetingPill} ${silent != null && silent !== "" ? styles.warning : ""}`}
      role="status"
    >
      <span aria-hidden="true" className={styles.dot} />
      <span className={styles.labelDetail}>{label}</span>
      <PillVoiceStatus className={styles.voice} meeting={meeting} />
      {ending && (
        <button
          className={styles.control}
          onClick={() => send({ command: "recording_keep", recording_id: ending.recording_id })}
        >
          {es.callDetection.keep}
        </button>
      )}
      {/* Always rendered: the stylesheet shows them on hover and on keyboard
          focus (:focus-within). Rendering them only on mouse hover left
          «Terminar» out of reach of the keyboard and of accessibility, which
          a run of the real test on the Mac found. */}
      <MeetingControls
        meeting={meeting}
        onAsk={() => setAsking(true)}
        send={send}
        silent={silent}
      />
      {asking && <PillAsk meeting={meeting} onDone={() => setAsking(false)} send={send} />}
    </div>
  );
}

function MeetingControls({
  meeting,
  onAsk,
  send,
  silent,
}: Pick<Parameters<typeof ActiveMeetingPill>[0], "meeting" | "send" | "silent"> & {
  onAsk: () => void;
}) {
  return (
    <span className={`${styles.controls} ${meeting.ask ? styles.withAsk : ""}`}>
      <PillAskButton className={styles.control} meeting={meeting} onAsk={onAsk} />
      {Boolean(silent) && (
        <TextLink className={styles.help} href="#/settings">
          {es.meeting.settingsHelp}
        </TextLink>
      )}
      {meeting.phase === "recording" && (
        <button className={styles.control} onClick={() => send({ command: "meeting_pause" })}>
          {es.meeting.pause}
        </button>
      )}
      {meeting.phase === "paused" && (
        <button className={styles.control} onClick={() => send({ command: "meeting_resume" })}>
          {es.meeting.resume}
        </button>
      )}
      <button className={styles.control} onClick={() => send({ command: "meeting_finish" })}>
        {es.meeting.finish}
      </button>
    </span>
  );
}

/** The recording in progress (Orbit's own problems are the status pill's). */
export function MeetingPill({ send, snapshot }: MeetingPillProps) {
  const now = useNow(snapshot.meeting.phase === "idle" ? null : 1_000);
  if (snapshot.meeting.phase === "idle") return null;

  const silent = silentLabel(snapshot.meeting);
  // Once the call ended, its countdown is what matters, even if the
  // call's audio has gone quiet.
  const panel = snapshot.meeting.call_panel;
  const label =
    panel?.state === "ending"
      ? endingLabel(panel.ends_at_ms, now)
      : (silent ?? statusLabel(snapshot.meeting, now));
  return <ActiveMeetingPill label={label} meeting={snapshot.meeting} send={send} silent={silent} />;
}
