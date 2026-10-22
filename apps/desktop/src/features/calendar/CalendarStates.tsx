import { useState, type ReactNode } from "react";
import { Button } from "../../design/Button";
import { EmptyState } from "../../design/EmptyState";
import type { IconName } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";
import { Loading } from "../../design/Spinner";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import styles from "./CalendarView.module.css";

/** What the calendar shows instead of itself: why, and the one action that fixes it. */
export function CalendarEmpty(props: {
  action?: ReactNode;
  children: ReactNode;
  error: string | null;
  hint?: string;
  icon?: IconName;
  title: string;
}) {
  return (
    <section className={styles.stateView}>
      <ViewError error={props.error} />
      <div className={styles.stateCard}>
        <EmptyState action={props.action} icon={props.icon ?? "calendar"} title={props.title}>
          {props.children}
        </EmptyState>
        {props.hint !== undefined && <p className={styles.stateHint}>{props.hint}</p>}
      </div>
    </section>
  );
}

/**
 * Never asked yet: «Permitir acceso al calendario» asks macOS, and says
 * «Esperando a macOS…» until macOS answers. macOS's own dialog is the only
 * decision; the answer replaces this view, so nothing here times out.
 */
export function ConnectCalendar(props: { error: string | null; onConnect: () => Promise<unknown> }) {
  const [asking, setAsking] = useState(false);
  return (
    <CalendarEmpty
      action={
        <Button
          feedback={asking ? "pending" : "idle"}
          onClick={() => {
            setAsking(true);
            props.onConnect().catch(() => setAsking(false));
          }}
          pendingLabel={es.calendar.connectWaiting}
          size={36}
          variant="accent"
        >
          {es.calendar.connect}
        </Button>
      }
      error={props.error}
      hint={es.calendar.connectHint}
      title={es.calendar.connectTitle}
    >
      {es.calendar.connectDescription}
    </CalendarEmpty>
  );
}

const DAY_COLUMNS = 7;
const HEADER_WIDTHS = ["40%"];

/**
 * Nothing known yet (the permission, the Mac's zone): the calendar's own
 * shape filling in, with what it is waiting for. Never a blank view, and never
 * «denied» before macOS says so.
 */
export function CalendarLoading({ error }: { error: string | null }) {
  return (
    <section aria-busy="true" className={styles.view}>
      <ViewError error={error} />
      <header className={styles.header}>
        <Loading size={16}>{es.calendar.loading}</Loading>
      </header>
      <div className={styles.body}>
        <div aria-hidden="true" className={styles.loadingGrid}>
          {Array.from({ length: DAY_COLUMNS }, (_, index) => (
            <div className={styles.loadingColumn} key={index}>
              <Skeleton height={11} widths={HEADER_WIDTHS} />
              <span className={styles.loadingBlock} style={{ top: `${18 + (index % 3) * 9}%` }} />
              <span className={styles.loadingBlock} style={{ top: `${52 + (index % 2) * 12}%` }} />
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
