import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { Button } from "../../design/Button";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { effectiveTimezone, isoWeekForDay, localDayIndex } from "../../local-time";
import { localDateLabel, localHour, nextMeeting, useBriefingClock } from "./briefing-clock";
import { BriefingSections, briefingAgendaStatus, type AgendaStatus } from "./BriefingSections";
import { BriefingWithoutZone } from "./BriefingWithoutZone";
import { NextMeetingCard } from "./NextMeetingCard";
import { useBriefingReads } from "./useBriefingReads";
import styles from "./BriefingView.module.css";

interface Props {
  error: string | null;
  onRecord: () => void;
  send: (command: AppCommand) => void;
  snapshot: AppSnapshot;
}

function greetingForHour(hour: number): string {
  if (hour < 14) return es.briefing.morning;
  if (hour < 21) return es.briefing.afternoon;
  return es.briefing.night;
}

function summaryParts(briefing: BriefingSnapshot): {
  beforeFocus: string;
  focusHours: string | null;
  focusMinutes: string;
  afterFocus: string;
} {
  const count = briefing.summary.meetings_today;
  const meetings = count === 1 ? es.briefing.summaryMeeting : es.briefing.summaryMeetings;
  const hours = Math.floor(briefing.summary.focus_minutes / 60);
  const minutes = briefing.summary.focus_minutes % 60;
  return {
    beforeFocus: `${count === 0 ? es.briefing.zeroMeetings : `${count} ${meetings}`} · `,
    focusHours: hours > 0 ? `${hours} h` : null,
    focusMinutes: minutes > 0 || hours === 0 ? `${minutes} min` : "",
    afterFocus: ` ${es.briefing.focus}`,
  };
}

export default function BriefingView({ error, onRecord, send, snapshot }: Props) {
  const nowMs = useBriefingClock();
  const timezone = effectiveTimezone(snapshot.system_timezone ?? "");
  useBriefingReads(snapshot.briefing, send);
  const greeting =
    timezone === null ? es.briefing.hello : greetingForHour(localHour(nowMs, timezone));
  const briefing = snapshot.briefing;
  if (!briefing) return <EmptyBriefing error={error} onRecord={onRecord} greeting={greeting} />;
  if (timezone === null)
    return (
      <BriefingWithoutZone
        briefing={briefing}
        error={error}
        onRecord={onRecord}
        snapshot={snapshot}
      />
    );
  const agendaStatus = briefingAgendaStatus(snapshot);
  const next = nextMeeting(briefing, nowMs);
  const noMeeting = agendaStatus.known
    ? briefing.agenda.length > 0
      ? es.briefing.noUpcoming
      : es.briefing.noEvents
    : agendaStatus.text;
  return (
    <div className={styles.page}>
      <ViewError error={error} />
      <div className={styles.content}>
        <BriefingHero
          briefing={briefing}
          greeting={greeting}
          agendaStatus={agendaStatus}
          dateLabel={localDateLabel(nowMs, timezone)}
          week={isoWeekForDay(localDayIndex(nowMs, timezone))}
        />
        <div className={styles.grid}>
          {next ? (
            <NextMeetingCard
              meeting={next}
              send={send}
              nowMs={nowMs}
              timezone={timezone}
              recordingState={briefing.sources?.recording}
            />
          ) : (
            <section className={styles.emptyMeeting}>{noMeeting}</section>
          )}
          <BriefingSections briefing={briefing} snapshot={snapshot} />
        </div>
      </div>
    </div>
  );
}

function BriefingHero({
  briefing,
  greeting,
  agendaStatus,
  dateLabel,
  week,
}: {
  briefing: BriefingSnapshot;
  greeting: string;
  agendaStatus: AgendaStatus;
  dateLabel: string;
  week: number;
}) {
  const name =
    briefing.greeting.name != null && briefing.greeting.name !== ""
      ? `, ${briefing.greeting.name}`
      : "";
  const summary = summaryParts(briefing);
  return (
    <header className={styles.hero}>
      <div className={styles.date}>
        {dateLabel} · {es.briefing.weekPrefix}
        {week}
      </div>
      <h1>
        {greeting}
        {name}
      </h1>
      {agendaStatus.known ? (
        <p>
          {summary.beforeFocus}
          {Boolean(summary.focusHours) && <span>{summary.focusHours}</span>}{" "}
          <span>{summary.focusMinutes}</span>
          {summary.afterFocus}
        </p>
      ) : (
        agendaStatus.text !== null && <p>{agendaStatus.text}</p>
      )}
    </header>
  );
}

function EmptyBriefing({
  error,
  onRecord,
  greeting,
}: {
  error: string | null;
  onRecord: () => void;
  greeting: string;
}) {
  return (
    <div className={styles.page}>
      <ViewError error={error} />
      <section className={styles.empty}>
        <h1>{greeting}</h1>
        <Button onClick={onRecord} variant="accent">
          {es.meeting.recordNow}
        </Button>
      </section>
    </div>
  );
}
