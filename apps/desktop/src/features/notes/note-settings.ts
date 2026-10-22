import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { es } from "../../i18n/es";
import { templateOf } from "./note-progress";
import { languageName } from "./notes-format";

// What «Cómo escribe Orbit esta nota» decides, apart from how it looks.

/** The languages a meeting can be picked in, in display order. */
export const LANGUAGES = ["es", "en", "ca", "fr", "pt", "de", "it"];

export interface Choice {
  template: string;
  language: string | null;
  /** A dictionary's id, or "" for the series' own. */
  dictionary: string;
  series: boolean;
}

function seriesUses(note: OpenNoteSnapshot, template: string): boolean {
  const assigned = note.series?.assigned;
  return assigned?.state === "known" && (assigned.template_id ?? "general") === template;
}

/** What the note is written with now, as the hub says. */
export function current(note: OpenNoteSnapshot): Choice {
  const template = templateOf(note);
  return {
    template,
    language: note.transcript.language,
    dictionary: note.dictionary?.dictionary_id ?? "",
    series: seriesUses(note, template),
  };
}

export function templateName(
  library: NoteLibrarySnapshot | null | undefined,
  id: string,
  note: OpenNoteSnapshot,
) {
  if (note.summary?.template_id === id) return note.summary.template_name;
  return library?.items.find((item) => item.id === id)?.name ?? id;
}

/** The trigger's words: «Plantilla: Revisión de diseño · Español». */
export function howLabel(note: OpenNoteSnapshot, library?: NoteLibrarySnapshot | null): string {
  const template = note.summary?.template_name ?? templateName(library, templateOf(note), note);
  const language = note.transcript.language;
  return es.noteScreen.howLabel(template, language === null ? null : languageName(language));
}

/**
 * The orders a choice gives, in the order the hub must take them: the series
 * first (template and dictionary in one transaction), then this meeting's
 * dictionary, then the note written again and the meeting transcribed again.
 */
export function orders(note: OpenNoteSnapshot, from: Choice, to: Choice): AppCommand[] {
  const commands: AppCommand[] = [];
  const recording_id = note.recording_id;
  const seriesChanged =
    to.series !== from.series ||
    (to.series && (to.template !== from.template || to.dictionary !== from.dictionary));
  if (note.series && seriesChanged && to.series) {
    const followed = note.dictionary?.source === "series" ? note.dictionary.dictionary_id : null;
    commands.push({
      command: "note_library",
      request: {
        action: "use_for_series",
        recording_id,
        template_id: to.template,
        dictionary_id: to.dictionary === "" ? followed : to.dictionary,
      },
    });
  } else if (note.series && seriesChanged) {
    // Off: the series goes back to General.
    commands.push({
      command: "note_library",
      request: {
        action: "assign",
        calendar_item_id: note.series.calendar_item_id,
        template_id: null,
      },
    });
  }
  if (!to.series && to.dictionary !== from.dictionary) {
    commands.push({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "select",
          recording_id,
          dictionary_id: to.dictionary === "" ? null : to.dictionary,
          use_for_series: false,
        },
      },
    });
  }
  if (to.template !== from.template)
    commands.push({ command: "note_regenerate", recording_id, template_id: to.template });
  if (to.language !== null && to.language !== from.language)
    commands.push({ command: "meeting_language", recording_id, language: to.language });
  return commands;
}

/** Why the hub did not take the last change for this meeting or its series. */
export function refusals(note: OpenNoteSnapshot, library?: NoteLibrarySnapshot | null): string[] {
  const series = note.series?.calendar_item_id;
  const mine = (library?.refusals ?? []).filter(
    (refusal) =>
      (refusal.target_kind === "meeting_series" && refusal.target_id === note.recording_id) ||
      (refusal.target_kind === "series" && refusal.target_id === series),
  );
  const dictionaries = (library?.dictionary_failures ?? []).filter(
    (failure) => failure.target === `dictionary_meeting:${note.recording_id}`,
  );
  return [
    ...mine.map(
      (refusal) =>
        es.noteScreen.seriesRefused(refusal.code) ??
        (refusal.code.startsWith("dictionar")
          ? es.dictionary.refused(refusal.code, "", refusal.numbers ?? {})
          : es.library.refused(refusal.code)),
    ),
    ...dictionaries.map((failure) =>
      es.dictionary.refused(failure.code, failure.detail, failure.numbers),
    ),
  ];
}
