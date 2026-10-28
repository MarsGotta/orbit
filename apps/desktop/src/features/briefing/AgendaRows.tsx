import { getCopy } from "../../i18n";
import { Fragment } from "react";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { BriefingAgendaSnapshot } from "../../bridge/generated/BriefingAgendaSnapshot";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";

import { useUiStore } from "../../state/ui";
import { timeLabel } from "../calendar/date";
import styles from "./BriefingView.module.css";

interface Props {
  briefing: BriefingSnapshot;
  meeting: AppSnapshot["meeting"] | undefined;
  nowMs: number;
  timezone: string;
}

const ended = (row: BriefingAgendaSnapshot, nowMs: number) =>
  row.start_ms + row.minutes * 60_000 <= nowMs;

/** What a row says on its right: recording, its note ready, or who goes. */
function datum(row: BriefingAgendaSnapshot, recording: boolean, noteReady: boolean): string {
  if (recording) return getCopy().briefing.recording;
  if (noteReady) return getCopy().briefing.noteReady;
  if (row.people_count !== undefined) return getCopy().briefing.peopleCount(row.people_count);
  return row.people_label;
}

/**
 * The day's meetings: what is past is dimmed, the next one stands out, and a
 * line with the hour marks now between them. Each row opens its event.
 */
export function AgendaRows({ briefing, meeting, nowMs, timezone }: Props) {
  const openCalendarEvent = useUiStore((state) => state.openCalendarEvent);
  const rows = briefing.agenda;
  if (rows.length === 0) return null;
  const recording =
    meeting !== undefined && meeting.phase !== "idle"
      ? new Set(
          [briefing.next, ...(briefing.upcoming ?? [])]
            .filter((item) => item?.recording === "recording")
            .map((item) => item?.event_id),
        )
      : new Set<string | undefined>();
  const ready = new Set(
    (meeting?.recent ?? [])
      .filter((recent) => recent.summary === "ready")
      .map((recent) => recent.calendar_event_id),
  );
  const firstAhead = rows.findIndex((row) => !ended(row, nowMs));
  const nowAt = firstAhead === -1 ? rows.length : firstAhead;
  const nextId = briefing.next?.event_id ?? rows[firstAhead]?.event_id;
  return (
    <ul className={styles.agendaRows}>
      {rows.map((row, index) => (
        <Fragment key={row.event_id}>
          {index === nowAt && nowAt > 0 && <NowLine label={timeLabel(nowMs, timezone)} />}
          <li>
            <button
              className={styles.agendaRow}
              data-color={row.color}
              data-next={row.event_id === nextId && !ended(row, nowMs)}
              data-past={ended(row, nowMs)}
              data-recording={recording.has(row.event_id)}
              onClick={() => openCalendarEvent(row.event_id)}
              type="button"
            >
              <time>{timeLabel(row.start_ms, timezone)}</time>
              <span aria-hidden="true" className={styles.dot} />
              <span className={styles.rowTitle}>{row.title}</span>
              <span className={styles.rowDatum}>
                {datum(row, recording.has(row.event_id), ready.has(row.event_id))}
              </span>
            </button>
          </li>
        </Fragment>
      ))}
      {nowAt === rows.length && <NowLine label={timeLabel(nowMs, timezone)} />}
    </ul>
  );
}

function NowLine({ label }: { label: string }) {
  return (
    <li aria-hidden="true" className={styles.nowLine}>
      <span>{label}</span>
      <span />
    </li>
  );
}
