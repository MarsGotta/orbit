import { dateTime } from "../../i18n/locale";
import { getCopy, lookupCopy } from "../../i18n";
import type { NoteDayGroup } from "../../bridge/generated/NoteDayGroup";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { mmss } from "../../time-format";

const DAY_MS = 86_400_000;

/** A turn's offset: mm:ss, or h:mm:ss from the first hour. */
export function formatTimestamp(atMs: number): string {
  const hours = Math.floor(Math.max(0, atMs) / 3_600_000);
  return hours > 0 ? `${hours}:${mmss(atMs % 3_600_000)}` : mmss(atMs);
}

const weekday = dateTime({ weekday: "short" });
const dayMonth = dateTime({ day: "numeric", month: "short" });
const clock = dateTime({
  hour: "numeric",
  minute: "2-digit",
});

/** «14 oct», «17 sept»: Intl's abbreviations without their full stop. */
export function formatDayMonth(value: number): string {
  return dayMonth.format(value).replace(".", "");
}

/** «Mié 14 oct». */
export function formatDay(value: number): string {
  const name = weekday.format(value).replace(".", "");
  return `${name.charAt(0).toUpperCase()}${name.slice(1)} ${formatDayMonth(value)}`;
}

/** «9:00», «17:10». */
export function formatClock(value: number): string {
  return clock.format(value);
}

/** A meeting's length: «47 min», «1 h 52 min». */
export function formatLength(durationMs: number): string {
  const minutes = Math.max(0, Math.round(durationMs / 60_000));
  if (minutes < 60) return getCopy().noteScreen.minutes(minutes);
  return getCopy().noteScreen.hours(Math.floor(minutes / 60), minutes % 60);
}

/** Where a meeting is in the list: its hour today and yesterday, its date before. */
export function rowTime(note: NoteListItem): string {
  const group = groupFor(note);
  return group === "today" || group === "yesterday"
    ? formatClock(note.started_at_ms)
    : formatDayMonth(note.started_at_ms);
}

/** The list's day groups, in order, with their names. */
export const dayGroups = (): Array<[NoteDayGroup, string]> => [
  ["today", getCopy().notes.today],
  ["yesterday", getCopy().notes.yesterday],
  ["this_week", getCopy().notes.thisWeek],
  ["last_30_days", getCopy().notes.last30Days],
  ["before", getCopy().notes.before],
];

export function groupFor(note: NoteListItem): NoteDayGroup {
  if (note.group) return note.group;
  const days = Math.max(0, Math.floor((Date.now() - note.started_at_ms) / DAY_MS));
  if (days === 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 7) return "this_week";
  if (days < 30) return "last_30_days";
  return "before";
}

/** Who speaks in a turn: the name the hub gave its speaker. */
export function speakerName(note: OpenNoteSnapshot, turn: NoteTurnSnapshot): string {
  return (
    note.speakers.find((speaker) => speaker.key === turn.speaker_key)?.name ?? turn.speaker_key
  );
}

/** The meeting's language, named in its own language; its code when not named. */
export function languageName(code: string): string {
  return lookupCopy(getCopy().notes.languages, code) ?? code;
}

/** The transcript as text, for «Copiar transcripción»: «Nombre: texto», one turn per line. */
export function transcriptText(note: OpenNoteSnapshot): string {
  return note.transcript.turns.map((turn) => `${speakerName(note, turn)}: ${turn.text}`).join("\n");
}

/** The summary as markdown, for «Copiar resumen». */
export function summaryMarkdown(note: OpenNoteSnapshot): string {
  const summary = note.summary;
  if (!summary) return "";
  const parts = [`# ${note.title}`, summary.short_summary];
  for (const section of summary.sections) parts.push(`## ${section.name}\n\n${section.markdown}`);
  if (note.decisions.length > 0)
    parts.push(
      `## ${getCopy().noteScreen.decisions}\n\n${note.decisions.map((item) => `- ${item}`).join("\n")}`,
    );
  if (note.open_topics.length > 0)
    parts.push(
      `## ${getCopy().noteScreen.openTopics}\n\n${note.open_topics.map((item) => `- ${item}`).join("\n")}`,
    );
  return parts.filter((part) => part.trim() !== "").join("\n\n");
}

/**
 * A voice nobody named yet: the hub's «Persona N» for `voice_N`, «Otra persona»
 * and the local microphone before the Mac said whose it is
 * (`server/orbit_hub/meetings/turns.py`). It asks for a name.
 */
export function isUnnamedSpeaker(speaker: { key: string; name: string } | undefined): boolean {
  if (!speaker) return true;
  if (speaker.name === "Otra persona" || speaker.name === "Micrófono local") return true;
  const voice = /^voice_(\d+)$/.exec(speaker.key);
  return voice !== null && speaker.name === `Persona ${Number(voice[1])}`;
}

/** How «Quién habló» names a person: the first name, a placeholder whole. */
export function shortName(speaker: { key: string; name: string }): string {
  return isUnnamedSpeaker(speaker) ? speaker.name : (speaker.name.split(/\s+/)[0] ?? speaker.name);
}
