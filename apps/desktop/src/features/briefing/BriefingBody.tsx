import type { AppView as AppSnapshot } from "../../state/app-view";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { Button } from "../../design/Button";
import { EmptyState } from "../../design/EmptyState";
import { es } from "../../i18n/es";
import { nextMeeting } from "./briefing-clock";
import { BriefingLoading } from "./BriefingLoading";
import { BriefingSections, briefingAgendaStatus, type AgendaStatus } from "./BriefingSections";
import { CalendarAccess } from "./CalendarAccess";
import { NextMeetingCard } from "./NextMeetingCard";
import { RecordingCard } from "./RecordingCard";
import type { Send } from "./send";
import styles from "./BriefingView.module.css";

interface Props {
  briefing: BriefingSnapshot;
  nowMs: number;
  onRecord: () => void;
  send: Send;
  snapshot: AppSnapshot;
  timezone: string;
}

/** Below the greeting: the meeting that matters now and the day's agenda. */
export function BriefingBody(props: Props) {
  const { briefing, nowMs, send, snapshot, timezone } = props;
  const status = briefingAgendaStatus(snapshot);
  const meeting = snapshot.meeting;
  const recording =
    meeting.phase === "idle" ? null : (
      <RecordingCard
        event={briefing.next?.recording === "recording" ? briefing.next : null}
        meeting={meeting}
        send={send}
        timezone={timezone}
      />
    );
  if (status.state !== "known")
    return (
      <>
        {recording}
        <AgendaUnknown send={send} status={status} />
      </>
    );
  const next = nextMeeting(briefing, nowMs);
  return (
    <>
      {recording ??
        (next ? (
          <NextMeetingCard
            meeting={next}
            nowMs={nowMs}
            online={snapshot.connection?.hub === "paired"}
            send={send}
            timezone={timezone}
          />
        ) : (
          <NoMeetings onRecord={props.onRecord} zero={briefing.agenda.length === 0} />
        ))}
      <BriefingSections briefing={briefing} nowMs={nowMs} snapshot={snapshot} />
    </>
  );
}

/** The agenda is not known: on its way, not allowed, stopped or refused. */
function AgendaUnknown({
  send,
  status,
}: {
  send: Send;
  status: Exclude<AgendaStatus, { state: "known" }>;
}) {
  switch (status.state) {
    case "reading":
      return <BriefingLoading />;
    case "permission":
      return <CalendarAccess authorization={status.authorization} send={send} />;
    case "blocked":
      return (
        <p className={`${styles.card} ${styles.notice}`}>{es.briefing.blocked[status.blocker]}</p>
      );
    case "failed":
      return (
        <p className={`${styles.card} ${styles.notice}`} role="alert">
          {es.briefing.agendaFailed}
          <small>{status.cause}</small>
        </p>
      );
  }
}

function NoMeetings({ onRecord, zero }: { onRecord: () => void; zero: boolean }) {
  return (
    <section className={styles.card}>
      <EmptyState
        action={
          <Button onClick={onRecord} size={36} variant="accent">
            {es.briefing.record}
          </Button>
        }
        icon="calendar"
        title={zero ? es.briefing.noMeetingsTitle : es.briefing.noneLeftTitle}
      >
        {es.briefing.noMeetingsText}
      </EmptyState>
    </section>
  );
}

/** Without the Mac's zone no hour can be placed: recording still can be. */
export function ZoneFailed({ onRecord }: { onRecord: () => void }) {
  return (
    <div className={styles.actions}>
      <Button onClick={onRecord} size={40} variant="accent">
        {es.briefing.record}
      </Button>
    </div>
  );
}
