import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { CallPanelSnapshot } from "../../bridge/generated/CallPanelSnapshot";
import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { secondsLeft } from "../../app/call-countdown";
import { DiscardButton, DiscardQuestion, useDiscardQuestion } from "../../app/DiscardRecording";
import { Button } from "../../design/Button";
import { useFeedback } from "../../design/feedback";

import { mmss } from "../../time-format";
import { useNow } from "../../use-now";
import styles from "./CallPanel.module.css";

/** A panel order; its promise settles with the daemon's receipt. */
export type PanelSend = (command: AppCommand) => Promise<unknown>;

type Offer = Extract<CallPanelSnapshot, { state: "offer" }>;
type Ending = Extract<CallPanelSnapshot, { state: "ending" }>;
type Recording = Extract<CallPanelSnapshot, { state: "recording" }>;

/** «Llamada en Google Meet», with its event: record it or not now. */
export function OfferCard({ panel, send }: { panel: Offer; send: PanelSend }) {
  const es = useCopy();
  const record = useFeedback();
  return (
    <section aria-label={es.callDetection.panelLabel} className={styles.card}>
      <p className={styles.title}>{es.callDetection.offer(panel.application)}</p>
      {panel.event_title != null && panel.event_title !== "" && (
        <p className={styles.sub}>{panel.event_title}</p>
      )}
      <div className={styles.actions}>
        <Button
          className={styles.grow}
          doneLabel={es.callDetection.recordPending}
          feedback={record.feedback}
          onClick={() => record.track(send({ command: "call_record", call_id: panel.call_id }))}
          pendingLabel={es.callDetection.recordPending}
          size={32}
          variant="danger"
        >
          {es.callDetection.record}
        </Button>
        <Button
          className={styles.grow}
          onClick={() => void send({ command: "call_dismiss", call_id: panel.call_id })}
          size={32}
        >
          {es.callDetection.notNow}
        </Button>
      </div>
    </section>
  );
}

/**
 * The call ended: the recording it started stops when the daemon's
 * countdown does, unless «Seguir grabando».
 */
export function EndingCard({ panel, send }: { panel: Ending; send: PanelSend }) {
  const es = useCopy();
  const now = useNow(1_000);
  const keep = useFeedback();
  return (
    <section aria-label={es.callDetection.panelLabel} className={styles.card}>
      <p className={styles.title}>{es.callDetection.ended}</p>
      <p className={`${styles.sub} ${styles.countdown}`} role="timer">
        {es.callDetection.endingIn(secondsLeft(panel.ends_at_ms, now))}
      </p>
      <Remaining nowMs={now} untilMs={panel.ends_at_ms} />
      <div className={styles.actions}>
        <Button
          className={styles.grow}
          doneLabel={es.callDetection.keepPending}
          feedback={keep.feedback}
          onClick={() =>
            keep.track(send({ command: "recording_keep", recording_id: panel.recording_id }))
          }
          pendingLabel={es.callDetection.keepPending}
          size={32}
        >
          {es.callDetection.keep}
        </Button>
      </div>
    </section>
  );
}

/** «Silenciar» / «Reactivar micrófono»: only her microphone, never the call. */
function MuteButton({ meeting, send }: { meeting: MeetingSnapshot; send: PanelSend }) {
  const copy = useCopy();
  const muted = meeting.microphone_muted === true;
  return (
    <Button onClick={() => void send({ command: "microphone_mute", muted: !muted })} size={32}>
      {muted ? copy.microphoneMute.unmute : copy.microphoneMute.mute}
    </Button>
  );
}

/**
 * Any recording in progress, started by a call or by hand: its time, what it
 * records, «Preguntar a Deimos» while asking is offered, «Silenciar», «Parar»,
 * and whether «Oye Deimos» listens.
 */
export function RecordingCard({
  call,
  meeting,
  onAsk,
  send,
  voice,
}: {
  call: Recording | null;
  meeting: MeetingSnapshot;
  onAsk?: () => void;
  send: PanelSend;
  voice: string | null;
}) {
  const es = useCopy();
  const now = useNow(1_000);
  const stop = useFeedback();
  const discard = useDiscardQuestion(meeting);
  const elapsed = mmss(now - (meeting.started_at_ms ?? now));
  const paused = meeting.phase === "paused";
  const what =
    call === null
      ? meeting.title
      : call.event_title != null && call.event_title !== ""
        ? call.event_title
        : es.callDetection.offer(call.application);
  return (
    <div className={styles.card}>
      <p className={styles.status} data-paused={paused || undefined}>
        <span aria-hidden="true" className={styles.dot} />
        <span role="timer">
          {paused ? es.callDetection.pausedAt(elapsed) : es.callDetection.recordingAt(elapsed)}
        </span>
      </p>
      {what != null && what !== "" && <p className={styles.sub}>{what}</p>}
      {meeting.microphone_muted === true && <p className={styles.sub}>{es.microphoneMute.muted}</p>}
      {discard.asking && meeting.recording_id !== null ? (
        <DiscardQuestion onKeep={discard.keep} recordingId={meeting.recording_id} send={send} />
      ) : (
        <PanelControls discard={discard} meeting={meeting} onAsk={onAsk} send={send} stop={stop} />
      )}
      {voice !== null && <p className={styles.hint}>{voice}</p>}
    </div>
  );
}

/**
 * «Preguntar a Deimos», «Silenciar», «Parar» and, under them, «Terminar sin
 * guardar».
 */
function PanelControls({
  discard,
  meeting,
  onAsk,
  send,
  stop,
}: {
  discard: ReturnType<typeof useDiscardQuestion>;
  meeting: MeetingSnapshot;
  onAsk?: () => void;
  send: PanelSend;
  stop: ReturnType<typeof useFeedback>;
}) {
  const es = useCopy();
  const copy = useCopy();
  return (
    <>
      <div className={styles.actions}>
        {onAsk && (
          <Button className={styles.grow} onClick={onAsk} size={32}>
            {copy.meetingQuestion.ask}
          </Button>
        )}
        {meeting.phase === "recording" && <MuteButton meeting={meeting} send={send} />}
        <Button
          doneLabel={es.callDetection.stopPending}
          feedback={stop.feedback}
          onClick={() => stop.track(send({ command: "meeting_finish" }))}
          pendingLabel={es.callDetection.stopPending}
          size={32}
          variant="dangerSoft"
        >
          {es.callDetection.stop}
        </Button>
      </div>
      {discard.offered && <DiscardButton onAsk={discard.ask} />}
    </>
  );
}

/**
 * A bar that empties until `untilMs`, from where it stood when first shown;
 * `edge` lays it along the card's bottom edge (the answer's 45 s).
 */
export function Remaining({
  edge = false,
  nowMs,
  untilMs,
}: {
  edge?: boolean;
  nowMs: number;
  untilMs: number;
}) {
  const [total] = useState(() => Math.max(1, untilMs - Date.now()));
  const left = Math.min(1, Math.max(0, (untilMs - nowMs) / total));
  return (
    <div aria-hidden="true" className={`${styles.track} ${edge ? styles.edge : ""}`}>
      <span className={styles.fill} style={{ transform: `scaleX(${left})` }} />
    </div>
  );
}
