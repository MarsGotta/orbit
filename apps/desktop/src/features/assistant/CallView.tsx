import { useId } from "react";
import type { CallSnapshot } from "../../bridge/generated/CallSnapshot";
import { Button } from "../../design/Button";
import { Loading } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
import { es } from "../../i18n/es";
import { mmss } from "../../time-format";
import { useNow } from "../../use-now";
import styles from "./Call.module.css";

/**
 * The limits of a call, in view before they are reached: the whole minutes
 * left before it hangs up at its ceiling, the user's silence that hangs it
 * up, and the shortcut that hangs up from any app.
 */
export function callLimits(call: CallSnapshot, nowMs: number): string {
  const minutesLeft = Math.max(0, Math.floor((call.expires_at_ms - nowMs) / 60_000));
  return es.call.limits(minutesLeft, call.idle_hangup_ms);
}

/** The bars that move only while the agent speaks; still while the user does. */
function Bars({ count, speaking }: { count: number; speaking: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={styles.bars}
      data-speaking={speaking}
      data-testid="call-bars"
    >
      {Array.from({ length: count }, (_, index) => (
        <i key={index} />
      ))}
    </span>
  );
}

/** Who speaks, and the stopwatch; «Llamando a Deimos…» until the call begins. */
function CallStatus({ call, now }: { call: CallSnapshot | null; now: number }) {
  const copy = useSpanish();
  if (call === null)
    return (
      <div className={styles.who}>
        <Loading size={14}>{copy.call.connecting}</Loading>
      </div>
    );
  return (
    <div className={styles.who} role="status">
      <span>
        {call.voice_unavailable === true
          ? copy.assistant.noHermesVoice
          : call.speaking
            ? copy.call.speaking
            : copy.call.listening}
      </span>
      <span className={styles.clock} role="timer">
        {mmss(now - call.started_at_ms)}
      </span>
    </div>
  );
}

interface CallProps {
  /** The call; null while «Llamar» waits for it to begin. */
  call: CallSnapshot | null;
  onHangUp: () => void;
}

/**
 * The call, calm: a circle whose bars move while the agent speaks, who
 * speaks, the stopwatch, the last question and answer, whether it sees the
 * screen, «Minimizar», «Colgar» and the limits.
 */
export function CallView({
  call,
  lastQuestion,
  onHangUp,
  onMinimise,
}: CallProps & { lastQuestion: string | null; onMinimise: () => void }) {
  const copy = useSpanish();
  const titleId = useId();
  const now = useNow(1_000);
  const answer = call?.last_line ?? null;
  return (
    <section aria-labelledby={titleId} className={styles.view}>
      <header className={styles.header}>
        <h2 id={titleId}>{copy.call.title}</h2>
        <Button onClick={onMinimise} size={32}>
          {copy.call.seeConversation}
        </Button>
      </header>
      <div className={styles.body}>
        <div className={styles.circle}>
          <Bars count={5} speaking={call?.speaking ?? false} />
        </div>
        <CallStatus call={call} now={now} />
        {(lastQuestion !== null || answer !== null) && (
          <div className={styles.last}>
            {lastQuestion !== null && <p className={styles.asked}>{copy.call.you(lastQuestion)}</p>}
            {answer !== null && <p className={styles.answer}>{answer}</p>}
          </div>
        )}
        <p className={styles.facts}>
          {call?.seeing_screen === true && (
            <>
              <span>{copy.call.seesScreen}</span>
              <span aria-hidden="true">·</span>
            </>
          )}
          <span>{copy.call.interrupt}</span>
        </p>
        <div className={styles.buttons}>
          <Button onClick={onMinimise} size={40}>
            {copy.design.collapseCall}
          </Button>
          <Button onClick={onHangUp} size={40} variant="danger">
            {copy.design.hangUp}
          </Button>
        </div>
        {call !== null && <p className={styles.limits}>{callLimits(call, now)}</p>}
      </div>
    </section>
  );
}

/** The call, minimised: a small card over the conversation with «Abrir» and «Colgar». */
export function MinimisedCall({ call, onHangUp, onOpen }: CallProps & { onOpen: () => void }) {
  const copy = useSpanish();
  const now = useNow(1_000);
  return (
    <section aria-label={copy.call.minimised} className={styles.mini}>
      <Bars count={3} speaking={call?.speaking ?? false} />
      <div className={styles.miniText}>
        <strong>{copy.design.inCall}</strong>
        {call === null ? (
          <span>{copy.call.connecting}</span>
        ) : (
          <span role="timer">{mmss(now - call.started_at_ms)}</span>
        )}
      </div>
      <Button onClick={onOpen} size={28} variant="ghost">
        {copy.design.expandCall}
      </Button>
      <Button onClick={onHangUp} size={28} variant="danger">
        {copy.design.hangUp}
      </Button>
    </section>
  );
}
