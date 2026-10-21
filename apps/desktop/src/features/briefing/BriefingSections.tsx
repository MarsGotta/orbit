import type { AppView as AppSnapshot } from "../../state/app-view";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { TextLink } from "../../design/TextLink";
import { es } from "../../i18n/es";
import { useUiStore } from "../../state/ui";
import { timeLabel } from "../calendar/date";
import styles from "./BriefingView.module.css";

interface Props {
  briefing: BriefingSnapshot;
  snapshot: AppSnapshot;
}

/**
 * Whether today's agenda is known. Unknown is never «no meetings» and says
 * nothing; only a missing permission has words: how to grant it.
 */
export type AgendaStatus = { known: true } | { known: false; text: string | null };

const unknown: AgendaStatus = { known: false, text: null };

export function BriefingSections({ briefing, snapshot }: Props) {
  return <AgendaSection briefing={briefing} snapshot={snapshot} />;
}

export function briefingAgendaStatus(snapshot: AppSnapshot): AgendaStatus {
  const source = snapshot.briefing?.sources?.agenda;
  if (source?.state === "present") return { known: true };
  const calendar = snapshot.calendar;
  if (calendar.authorization === "denied" || calendar.reading.blocker === "calendar_permission")
    return { known: false, text: es.briefing.calendarPermission };
  if (source !== undefined) return unknown;
  return calendar.authorization === "granted" &&
    calendar.reading.state === "available" &&
    !calendar.stale
    ? { known: true }
    : unknown;
}

function AgendaSection({ briefing, snapshot }: Pick<Props, "briefing" | "snapshot">) {
  const status = briefingAgendaStatus(snapshot);
  const openCalendarEvent = useUiStore((state) => state.openCalendarEvent);
  const empty = status.known ? es.briefing.noEvents : status.text;
  return (
    <section aria-label={es.briefing.agenda} className={`${styles.panel} ${styles.agenda}`}>
      <header>
        <h2>{es.briefing.agenda}</h2>
        <TextLink href="#/calendar">{es.briefing.week}</TextLink>
      </header>
      {briefing.agenda.length === 0 ? (
        empty !== null && <p className={styles.emptyCopy}>{empty}</p>
      ) : (
        <ul className={styles.agendaRows}>
          {briefing.agenda.map((row) => {
            return (
              <li aria-label={row.title} key={row.event_id}>
                <time>{timeLabel(row.start_ms, snapshot.system_timezone)}</time>
                <span className={styles.eventColor} data-color={row.color} />
                <button onClick={() => openCalendarEvent(row.event_id)} type="button">
                  {row.title}
                </button>
                <span>
                  {row.people_count === undefined
                    ? row.people_label
                    : es.briefing.peopleCount(row.people_count)}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
