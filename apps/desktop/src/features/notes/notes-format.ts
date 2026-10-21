import type { NoteDayGroup } from "../../bridge/generated/NoteDayGroup";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { mmss } from "../../time-format";
import { es } from "../../i18n/es";

const DAY_MS = 86_400_000;

export function formatDuration(durationMs: number): string {
  const hours = Math.floor(Math.max(0, durationMs) / 3_600_000);
  return hours > 0 ? `${hours}:${mmss(durationMs % 3_600_000)}` : mmss(durationMs, false);
}

/** A turn's offset: mm:ss, or h:mm:ss from the first hour. */
export function formatTimestamp(atMs: number): string {
  const hours = Math.floor(Math.max(0, atMs) / 3_600_000);
  return hours > 0 ? `${hours}:${mmss(atMs % 3_600_000)}` : mmss(atMs);
}

export function formatDay(value: number): string {
  return new Intl.DateTimeFormat("es-ES", {
    weekday: "short",
    day: "numeric",
    month: "short",
  }).format(value);
}

export function formatClock(value: number): string {
  return new Intl.DateTimeFormat("es-ES", { hour: "2-digit", minute: "2-digit" }).format(value);
}

export function formatDate(value: number): string {
  return new Intl.DateTimeFormat("es-ES", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(value);
}

/** The list's day groups, in order, with their names (P9-04). */
export const DAY_GROUPS: Array<[NoteDayGroup, string]> = [
  ["today", es.notes.today],
  ["yesterday", es.notes.yesterday],
  ["this_week", es.notes.thisWeek],
  ["last_30_days", es.notes.last30Days],
  ["before", es.notes.before],
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

/** Who speaks in a turn: the name the hub gave its speaker (SIM-40). */
export function speakerName(note: OpenNoteSnapshot, turn: NoteTurnSnapshot): string {
  return (
    note.speakers.find((speaker) => speaker.key === turn.speaker_key)?.name ?? turn.speaker_key
  );
}

/** The meeting's language, named in its own language; its code when not named. */
export function languageName(code: string): string {
  return es.notes.languages[code] ?? code;
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
      `## ${es.notes.decisions}\n\n${note.decisions.map((item) => `- ${item}`).join("\n")}`,
    );
  if (note.open_topics.length > 0)
    parts.push(
      `## ${es.notes.openTopics}\n\n${note.open_topics.map((item) => `- ${item}`).join("\n")}`,
    );
  return parts.filter((part) => part.trim() !== "").join("\n\n");
}

const SPEAKER_COLORS = [
  "var(--o-accent)",
  "var(--o-blue)",
  "var(--o-green)",
  "var(--o-orange)",
  "var(--o-accent-ink)",
];

/** A colour that stays with a name across turns and notes. Red is left out:
 * it means recording and failure on this screen. */
export function stableColor(name: string): string {
  let hash = 0;
  for (const char of name) hash = (hash * 31 + (char.codePointAt(0) ?? 0)) >>> 0;
  return SPEAKER_COLORS[hash % SPEAKER_COLORS.length] ?? "var(--o-accent)";
}
