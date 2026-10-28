import { useCopy } from "../../i18n/CopyProvider";
import type { CSSProperties } from "react";
import type { BriefingAgendaSnapshot } from "../../bridge/generated/BriefingAgendaSnapshot";

import { timeLabel } from "../calendar/date";
import { localMinutes } from "./briefing-clock";
import styles from "./BriefingView.module.css";

// The band Today draws: a working day, 8:00 to 20:00 (Main.dc.html).
const FROM = 8 * 60;
const SPAN = 12 * 60;
const TICKS = [
  { label: "8:00", at: 0 },
  { label: "12:00", at: 4 / 12 },
  { label: "16:00", at: 8 / 12 },
  { label: "20:00", at: 1 },
];

/** Where `minutes` after midnight falls on the band, from 0 to 100 %. */
function percent(minutes: number): number {
  return Math.min(100, Math.max(0, ((minutes - FROM) / SPAN) * 100));
}

/**
 * The day as a thin line: each meeting a stretch in its calendar's colour,
 * what is past fainter, and the mark of now while the day is on the band.
 */
export function DayTimeline(props: {
  agenda: BriefingAgendaSnapshot[];
  nowMs: number;
  timezone: string;
}) {
  const es = useCopy();
  const { agenda, nowMs, timezone } = props;
  const now = localMinutes(nowMs, timezone);
  const onBand = now >= FROM && now <= FROM + SPAN;
  return (
    <div
      aria-label={es.briefing.dayLine(onBand ? timeLabel(nowMs, timezone) : null)}
      className={styles.timeline}
      role="img"
    >
      <span className={styles.track} />
      {agenda.map((row) => {
        const start = localMinutes(row.start_ms, timezone);
        const left = percent(start);
        const width = percent(start + row.minutes) - left;
        if (width <= 0) return null;
        return (
          <span
            className={styles.slot}
            data-color={row.color}
            data-past={row.start_ms + row.minutes * 60_000 <= nowMs}
            key={row.event_id}
            style={{ left: `${left}%`, width: `${width}%` }}
          />
        );
      })}
      {onBand && <span className={styles.nowMark} style={{ left: `${percent(now)}%` }} />}
      {TICKS.map((tick) => (
        <span
          className={styles.tick}
          data-edge={tick.at === 0 ? "start" : tick.at === 1 ? "end" : undefined}
          key={tick.label}
          style={{ "--at": `${tick.at * 100}%` } as CSSProperties}
        >
          {tick.label}
        </span>
      ))}
    </div>
  );
}
