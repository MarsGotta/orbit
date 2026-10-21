import { useEffect, useState } from "react";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";

export function localHour(nowMs: number, timezone: string): number {
  const hour = new Intl.DateTimeFormat("en-US", {
    hour: "numeric",
    hourCycle: "h23",
    timeZone: timezone,
  })
    .formatToParts(nowMs)
    .find((part) => part.type === "hour")?.value;
  return Number(hour ?? 0);
}

export function localDateLabel(nowMs: number, timezone: string): string {
  const date = new Intl.DateTimeFormat("es-ES", {
    weekday: "long",
    day: "numeric",
    month: "long",
    timeZone: timezone,
  })
    .format(nowMs)
    .replace(",", "");
  return date.charAt(0).toUpperCase() + date.slice(1);
}

export function useBriefingClock(): number {
  const [nowMs, setNowMs] = useState(Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  return nowMs;
}

export function nextMeeting(briefing: BriefingSnapshot, nowMs: number) {
  return (briefing.upcoming ?? (briefing.next ? [briefing.next] : [])).find(
    (meeting) => meeting.end_ms > nowMs,
  );
}
