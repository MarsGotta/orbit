import { useEffect, useRef, useState } from "react";
import { useNow } from "../use-now";
import { mmss } from "../time-format";
import { es } from "../i18n/es";
import { useSpanish } from "../i18n/AssistantCopy";
import { Icon } from "./Icon";
import styles from "./Orb.module.css";

type OrbState = "hidden" | "collapsed" | "expanded";

function WaveBars({ expanded = false }: { expanded?: boolean }) {
  const count = expanded ? 5 : 4;
  return (
    <span aria-hidden="true" className={styles.wave}>
      {Array.from({ length: count }, (_, index) => (
        <span className={styles.bar} key={index} />
      ))}
    </span>
  );
}

function useDragPosition() {
  const initialWidth = typeof window === "undefined" ? 1440 : window.innerWidth;
  const initialHeight = typeof window === "undefined" ? 900 : window.innerHeight;
  const [position, setPosition] = useState({ x: initialWidth - 110, y: initialHeight - 140 });
  const drag = useRef<{ dx: number; dy: number } | null>(null);
  useEffect(() => {
    const move = (event: MouseEvent) => {
      if (!drag.current) return;
      setPosition({ x: event.clientX - drag.current.dx, y: event.clientY - drag.current.dy });
    };
    const up = () => {
      drag.current = null;
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, []);
  const start = (event: React.MouseEvent) => {
    drag.current = { dx: event.clientX - position.x, dy: event.clientY - position.y };
  };
  return { position, start };
}

export interface OrbProps {
  endLabel?: string;
  lastLine?: string | null;
  onEnd: () => void;
  seeingScreen?: boolean;
  startedAtMs?: number;
  voiceUnavailable?: boolean;
}

interface ExpandedOrbProps {
  collapse: () => void;
  end: () => void;
  endLabel: string;
  lastLine: string | null;
  seeingScreen: boolean;
  startedAtMs: number;
  voiceUnavailable: boolean;
}

function ExpandedOrb(props: ExpandedOrbProps) {
  const es = useSpanish();
  const elapsed = mmss(Math.max(0, useNow(1_000) - props.startedAtMs));
  return (
    <section aria-label={es.design.inCall} className={styles.expanded} role="dialog">
      <header className={styles.header}>
        <WaveBars expanded />
        <strong>{es.design.inCall}</strong>
        <time>{elapsed}</time>
      </header>
      {props.lastLine && (
        <p>
          <strong>{es.chat.roles.assistant} · </strong>
          {props.lastLine}
        </p>
      )}
      {props.voiceUnavailable && <p className={styles.voiceNotice}>{es.assistant.noHermesVoice}</p>}
      <footer className={styles.orbFooter}>
        {props.seeingScreen && (
          <span className={styles.screenStatus}>
            <i />
            {es.assistant.watchingScreen}
          </span>
        )}
        <span className={styles.footerSpacer} />
        <button aria-label={es.design.collapseCall} onClick={props.collapse} type="button">
          <Icon name="minus" size={12} />
        </button>
        <button
          aria-label={props.endLabel}
          className={styles.hangup}
          onClick={props.end}
          type="button"
        >
          <Icon name="close" size={13} />
        </button>
      </footer>
    </section>
  );
}

export function Orb({
  endLabel = es.design.hangUp,
  lastLine = null,
  onEnd,
  seeingScreen = false,
  startedAtMs = Date.now(),
  voiceUnavailable = false,
}: OrbProps) {
  const [state, setState] = useState<OrbState>("expanded");
  const { position, start } = useDragPosition();
  if (state === "hidden") return null;
  const end = () => {
    setState("hidden");
    onEnd();
  };
  return (
    <div
      className={styles.position}
      onMouseDown={start}
      style={{ left: position.x, top: position.y }}
    >
      {state === "collapsed" ? (
        <button
          aria-label={es.design.expandCall}
          className={styles.collapsed}
          onClick={() => setState("expanded")}
          type="button"
        >
          <span aria-hidden="true" className={styles.ring} />
          <WaveBars />
        </button>
      ) : (
        <ExpandedOrb
          collapse={() => setState("collapsed")}
          end={end}
          endLabel={endLabel}
          lastLine={lastLine}
          seeingScreen={seeingScreen}
          startedAtMs={startedAtMs}
          voiceUnavailable={voiceUnavailable}
        />
      )}
    </div>
  );
}
