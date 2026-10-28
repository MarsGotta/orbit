import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import { Button } from "../../design/Button";
import { EmptyState } from "../../design/EmptyState";

import type { Send } from "./send";
import styles from "./BriefingView.module.css";

/**
 * Hoy without the calendar (Estados.dc.html, tile 09). macOS asks only
 * once: before that, «Permitir acceso al calendario» asks it and waits for
 * its answer; once denied, the way back is in System Settings.
 */
export function CalendarAccess({
  authorization,
  send,
}: {
  authorization: "not_determined" | "denied";
  send: Send;
}) {
  const macPermissions = useCopy().extras.macPermissions;
  const es = useCopy();
  const [asked, setAsked] = useState(false);
  const denied = authorization === "denied";
  const ask = () => {
    setAsked(true);
    const order = send({ command: "calendar", request: { action: "request_permission" } });
    if (order instanceof Promise) order.catch(() => setAsked(false));
  };
  const action = denied ? (
    <Button
      onClick={() =>
        void send({ command: "open_system_privacy", pane: macPermissions.calendar.pane })
      }
      size={36}
      variant="accent"
    >
      {es.briefing.openSystemSettings}
    </Button>
  ) : (
    <Button
      aria-busy={asked || undefined}
      disabled={asked}
      onClick={ask}
      size={36}
      variant="accent"
    >
      {asked ? es.controls.waitingForMacos : es.briefing.calendarAllow}
    </Button>
  );
  return (
    <section className={`${styles.card} ${styles.access}`}>
      <EmptyState action={action} icon="calendar" title={es.briefing.calendarTitle}>
        {denied ? es.briefing.calendarDenied : es.briefing.calendarText}
      </EmptyState>
      {!denied && <p className={styles.hint}>{es.briefing.calendarHint}</p>}
    </section>
  );
}
