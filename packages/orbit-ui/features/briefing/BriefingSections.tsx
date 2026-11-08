import { useCopy } from "../../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import type { CalendarReadingBlocker } from "../../bridge/generated/CalendarReadingBlocker";
import { TextLink } from "../../design/TextLink";

import { AgendaRows } from "./AgendaRows";
import { DayTimeline } from "./DayTimeline";
import styles from "./BriefingView.module.css";

interface Props {
  briefing: BriefingSnapshot;
  snapshot: AppSnapshot;
  /** The present instant; the suite's pinned clock when not given. */
  nowMs?: number;
}

/**
 * Whether today's agenda is known. Unknown is never «no meetings»: it is
 * being read, or a missing permission, a stop or a refusal says why not.
 */
export type AgendaStatus =
  | { state: "known" }
  | { state: "reading" }
  | { state: "permission"; authorization: "not_determined" | "denied" }
  | { state: "blocked"; blocker: Exclude<CalendarReadingBlocker, "calendar_permission"> }
  | { state: "failed"; cause: string };

export function briefingAgendaStatus(snapshot: AppSnapshot): AgendaStatus {
  const source = snapshot.briefing?.sources?.agenda;
  if (source?.state === "present") return { state: "known" };
  const calendar = snapshot.calendar;
  const blocker = calendar.reading.blocker;
  if (calendar.authorization === "denied") return { state: "permission", authorization: "denied" };
  if (calendar.authorization === "not_determined" || blocker === "calendar_permission")
    return { state: "permission", authorization: "not_determined" };
  if (blocker !== undefined) return { state: "blocked", blocker };
  if (source?.state === "absent") return { state: "failed", cause: source.cause };
  if (source !== undefined) return { state: "reading" };
  return calendar.authorization === "granted" &&
    calendar.reading.state === "available" &&
    !calendar.stale
    ? { state: "known" }
    : { state: "reading" };
}

/** «Agenda de hoy»: the day from 8:00 to 20:00 and its meetings. */
export function BriefingSections({ briefing, snapshot, nowMs = Date.now() }: Props) {
  const es = useCopy();
  const timezone = snapshot.system_timezone ?? "UTC";
  const meeting = snapshot.meeting as AppSnapshot["meeting"] | undefined;
  // While a meeting records, the card above holds the day's moment: the
  // list is enough (Grabando.dc.html draws no line of the day).
  const recording = meeting !== undefined && meeting.phase !== "idle";
  return (
    <section aria-labelledby="hoy-agenda" className={`${styles.card} ${styles.agenda}`}>
      <header className={styles.agendaHeader}>
        <h2 id="hoy-agenda">{es.briefing.agenda}</h2>
        <TextLink href="#/calendar">{es.briefing.seeWeek}</TextLink>
      </header>
      {!recording && <DayTimeline agenda={briefing.agenda} nowMs={nowMs} timezone={timezone} />}
      <AgendaRows briefing={briefing} meeting={meeting} nowMs={nowMs} timezone={timezone} />
    </section>
  );
}
