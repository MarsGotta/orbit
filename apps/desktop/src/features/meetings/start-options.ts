import type { MeetingAudio } from "../../bridge/generated/MeetingAudio";
import { effectiveTimezone } from "../../local-time";
import type { AppView } from "../../state/app-view";
import { es } from "../../i18n/es";

export type AudioSource = "system" | "microphone" | "application";

/** What `meeting_start` records, as orbit-core names it. */
export function selectedAudio(source: AudioSource, pid: number | null): MeetingAudio | null {
  if (source === "system") return "system";
  if (source === "microphone") return "microphone_only";
  return pid === null ? null : { application: { application_pid: pid } };
}

/** 9:05, in the zone the Mac published (the ambient one if it published none). */
export function clock(atMs: number, publishedZone: string): string {
  const timeZone = effectiveTimezone(publishedZone) ?? undefined;
  return new Intl.DateTimeFormat("es-ES", { hour: "numeric", minute: "2-digit", timeZone }).format(
    atMs,
  );
}

// An event further ahead than an hour is another meeting, not the one being
// recorded now; within it, its title is a likely name for this one. It limits
// only the suggestion: any title can be typed.
const SUGGEST_AHEAD_MS = 60 * 60_000;

/** «Usar «…», que empieza en 20 min»: the event in progress or about to start. */
export function titleSuggestion(
  snapshot: AppView,
  nowMs: number,
): { title: string; label: string } | null {
  const next = snapshot.briefing?.next;
  if (!next || next.title.trim() === "" || next.end_ms <= nowMs) return null;
  if (next.start_ms <= nowMs) {
    const started = clock(next.start_ms, snapshot.system_timezone);
    return { title: next.title, label: es.meeting.suggestStarted(next.title, started) };
  }
  if (next.start_ms - nowMs > SUGGEST_AHEAD_MS) return null;
  const minutes = Math.ceil((next.start_ms - nowMs) / 60_000);
  return { title: next.title, label: es.meeting.suggestStarts(next.title, minutes) };
}

/** What stops any recording now, said once as a row with the button that fixes it. */
export type StartBlocker = "privacy_stopped" | "microphone" | "screen";

export function startBlocker(snapshot: AppView, source: AudioSource): StartBlocker | null {
  if (snapshot.privacy_stopped) return "privacy_stopped";
  if (snapshot.permissions.microphone === "denied") return "microphone";
  if (source !== "microphone" && snapshot.permissions.screen === "denied") return "screen";
  return null;
}
