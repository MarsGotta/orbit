import type { AppCommand } from "../bridge/generated";
import type { CallSnapshot } from "../bridge/generated/CallSnapshot";
import { Button } from "../design/Button";
import { useCopy } from "../i18n/CopyProvider";
import { mmss } from "../time-format";
import { useNow } from "../use-now";
import styles from "./SidebarCard.module.css";

/** Whole minutes until the call hangs up by itself, rounded down as the
 *  call view counts them; the last minute reads «queda 1 min». */
function minutesLeft(expiresAtMs: number, now: number): number {
  return Math.max(1, Math.floor((expiresAtMs - now) / 60_000));
}

/**
 * The call with the agent, at the top of the sidebar in every view,
 * however it began (in the app or with ⌃⌥D): who, for how long, whether it
 * sees the screen, how long it may last, and «Colgar».
 */
export function CallCard({
  agentName,
  call,
  send,
}: {
  agentName: string;
  call: CallSnapshot;
  send: (command: AppCommand) => void;
}) {
  const es = useCopy();
  const now = useNow(1_000);
  const screen = call.seeing_screen ? es.callCard.seesScreen : es.callCard.noScreen;
  return (
    <section aria-label={es.callCard.label} className={styles.card}>
      <div className={styles.head}>
        <span className={styles.state} data-phase="call">
          {es.callCard.inCall}
        </span>
        <span aria-label={es.callCard.elapsed} className={styles.clock} role="timer">
          {mmss(now - call.started_at_ms)}
        </span>
      </div>
      <div className={styles.title}>{agentName}</div>
      <div className={styles.meta}>
        {screen} · {es.callCard.left(minutesLeft(call.expires_at_ms, now))}
      </div>
      <Button
        className={styles.wide}
        onClick={() => send({ command: "call_stop" })}
        size={32}
        variant="dangerSoft"
      >
        {es.callCard.hangUp}
      </Button>
    </section>
  );
}
