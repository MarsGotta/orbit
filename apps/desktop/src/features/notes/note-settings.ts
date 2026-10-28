import { getCopy } from "../../i18n";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";

import { meetingDictionaries, takesLists } from "./dictionary-choice";
import { templateOf } from "./note-progress";
import { languageName } from "./notes-format";

// What «Cómo escribe Orbit esta nota» decides, apart from how it looks.

/** The languages a meeting can be picked in, in display order. */
export const LANGUAGES = ["es", "en", "ca", "fr", "pt", "de", "it"];

export interface Choice {
  template: string;
  language: string | null;
  /** The meeting's dictionaries, in the hub's order. */
  dictionaries: string[];
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
    dictionaries: meetingDictionaries(note) ?? [],
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

/** The trigger's words: «Plantilla: Revisión de diseño · Español · 2 diccionarios». */
export function howLabel(note: OpenNoteSnapshot, library?: NoteLibrarySnapshot | null): string {
  const template = note.summary?.template_name ?? templateName(library, templateOf(note), note);
  const language = note.transcript.language;
  return getCopy().noteScreen.howLabel(
    template,
    language === null ? null : languageName(language),
    meetingDictionaries(note)?.length ?? 0,
  );
}

const same = (one: string[], other: string[]) => one.join("\n") === other.join("\n");

/** Whether the choice writes the note again: another template, language or dictionaries. */
export function rewrites(from: Choice, to: Choice): boolean {
  return (
    to.template !== from.template ||
    to.language !== from.language ||
    !same(to.dictionaries, from.dictionaries)
  );
}

/**
 * The orders a choice gives, in the order the hub must take them: the series
 * first (template and dictionaries in one transaction), then this meeting's
 * dictionaries, then the note written again and the meeting transcribed
 * again. A hub that takes lists gets the whole list; an older one, the first.
 */
export function orders(note: OpenNoteSnapshot, from: Choice, to: Choice): AppCommand[] {
  const commands: AppCommand[] = [];
  const recording_id = note.recording_id;
  const lists = takesLists(note);
  const dictionariesChanged = !same(to.dictionaries, from.dictionaries);
  const seriesChanged =
    to.series !== from.series ||
    (to.series && (to.template !== from.template || dictionariesChanged));
  if (note.series && seriesChanged && to.series) {
    commands.push({
      command: "note_library",
      request: {
        action: "use_for_series",
        recording_id,
        template_id: to.template,
        dictionary_id: lists ? null : (to.dictionaries[0] ?? null),
        ...(lists ? { dictionary_ids: to.dictionaries } : {}),
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
  if (!to.series && dictionariesChanged) {
    commands.push({
      command: "note_library",
      request: {
        action: "dictionary",
        request: {
          action: "select",
          recording_id,
          dictionary_id: lists ? null : (to.dictionaries[0] ?? null),
          ...(lists ? { dictionary_ids: to.dictionaries } : {}),
          use_for_series: false,
        },
      },
    });
  }
  if (to.template !== from.template || dictionariesChanged)
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
        getCopy().noteScreen.seriesRefused(refusal.code) ??
        (refusal.code.startsWith("dictionar")
          ? getCopy().dictionary.refused(refusal.code, "", refusal.numbers ?? {})
          : getCopy().library.refused(refusal.code, refusal.numbers)),
    ),
    ...dictionaries.map((failure) =>
      getCopy().dictionary.refused(failure.code, failure.detail, failure.numbers),
    ),
  ];
}
