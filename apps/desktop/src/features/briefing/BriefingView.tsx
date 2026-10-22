import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { effectiveTimezone, isoWeekForDay, localDayIndex } from "../../local-time";
import { localDateLabel, localHour, useBriefingClock } from "./briefing-clock";
import { BriefingBody, ZoneFailed } from "./BriefingBody";
import { BriefingLoading } from "./BriefingLoading";
import { briefingAgendaStatus } from "./BriefingSections";
import { useBriefingReads } from "./useBriefingReads";
import styles from "./BriefingView.module.css";

interface Props {
  error: string | null;
  onRecord: () => void;
  send: (command: AppCommand) => Promise<unknown>;
  snapshot: AppSnapshot;
}

function greetingForHour(hour: number): string {
  if (hour < 14) return es.briefing.morning;
  if (hour < 21) return es.briefing.afternoon;
  return es.briefing.night;
}

/**
 * Hoy: a column of 860 px with the day, the next meeting (or the one being
 * recorded) and the day's agenda. What is still on its way is drawn as it
 * will arrive; nothing is left blank.
 */
export default function BriefingView({ error, onRecord, send, snapshot }: Props) {
  const nowMs = useBriefingClock();
  const timezone = effectiveTimezone(snapshot.system_timezone ?? "");
  useBriefingReads(snapshot.briefing, send);
  const briefing = snapshot.briefing ?? null;
  const zoneFailed = timezone === null && (snapshot.system_timezone_error ?? "") !== "";
  return (
    <div className={styles.page}>
      <ViewError error={error} />
      <div className={styles.column}>
        <header className={styles.hero}>
          {timezone !== null && (
            <div className={styles.date}>
              {`${localDateLabel(nowMs, timezone)} · ${es.briefing.week(
                isoWeekForDay(localDayIndex(nowMs, timezone)),
              )}`}
            </div>
          )}
          <h1>
            {timezone === null ? es.briefing.hello : greetingForHour(localHour(nowMs, timezone))}
            {greetingName(briefing)}
          </h1>
          <DaySentence briefing={briefing} snapshot={snapshot} zoneFailed={zoneFailed} />
        </header>
        {timezone === null || briefing === null ? (
          zoneFailed ? (
            <ZoneFailed onRecord={onRecord} />
          ) : (
            <BriefingLoading />
          )
        ) : (
          <BriefingBody
            briefing={briefing}
            nowMs={nowMs}
            onRecord={onRecord}
            send={send}
            snapshot={snapshot}
            timezone={timezone}
          />
        )}
      </div>
    </div>
  );
}

function greetingName(briefing: BriefingSnapshot | null): string {
  const name = briefing?.greeting.name;
  return name != null && name !== "" ? `, ${name}` : "";
}

/** «Hoy tienes 4 reuniones y 2 h 45 min libres para concentrarte», once known. */
function DaySentence(props: {
  briefing: BriefingSnapshot | null;
  snapshot: AppSnapshot;
  zoneFailed: boolean;
}) {
  const { briefing, snapshot, zoneFailed } = props;
  if (zoneFailed) return <p>{es.briefing.timezoneFailed}</p>;
  if (briefing === null || briefingAgendaStatus(snapshot).state !== "known") return null;
  return (
    <p>
      {es.briefing.daySentence(briefing.summary.meetings_today, briefing.summary.focus_minutes)}
    </p>
  );
}
