import { useCopy } from "../i18n/CopyProvider";
import { useEffect, useState } from "react";
import { Button } from "../design/Button";
import { Icon } from "../design/Icon";
import { Spinner } from "../design/Spinner";

import styles from "./AppStates.module.css";

/**
 * How long the window waits for Orbit's first state before it says what is
 * happening: the promise its waiting screen makes («Si tarda más de 15
 * segundos…»). That state is local and comes in well under a second, so a
 * window still waiting after this is stuck, not slow. It does not end the
 * wait: the window keeps listening, and paints as soon as the state comes.
 */
export const SLOW_START_MS = 15_000;

/** Before Orbit's first state: connecting, and past the wait, what to do. */
export function Starting({ onClose }: { onClose: () => void }) {
  const es = useCopy();
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const timer = window.setTimeout(() => setSlow(true), SLOW_START_MS);
    return () => window.clearTimeout(timer);
  }, []);
  if (slow)
    return (
      <main className={styles.screen}>
        <div className={styles.box} role="alert">
          <span className={styles.warn}>
            <Icon name="warning" size={20} />
          </span>
          <h1>{es.appState.slow}</h1>
          <p>{es.appState.slowText}</p>
          <Button onClick={onClose} size={36}>
            {es.appState.close}
          </Button>
        </div>
      </main>
    );
  return (
    <main aria-busy="true" className={styles.screen}>
      <div className={styles.box} role="status">
        <Spinner size={20} />
        <h1>{es.appState.starting}</h1>
        <p>{es.appState.startingHint(SLOW_START_MS / 1_000)}</p>
      </div>
    </main>
  );
}

/** Orbit stopped answering, or the window could not reach it at all. */
export function NotResponding({ onClose }: { onClose: () => void }) {
  const es = useCopy();
  return (
    <main className={styles.screen}>
      <div className={styles.box} role="alert">
        <span className={styles.warn}>
          <Icon name="warning" size={20} />
        </span>
        <h1>{es.appState.lost}</h1>
        <p>{es.appState.lostText}</p>
        <Button onClick={onClose} size={36}>
          {es.appState.close}
        </Button>
      </div>
    </main>
  );
}
