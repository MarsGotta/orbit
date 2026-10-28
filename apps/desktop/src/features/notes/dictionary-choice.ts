import { getCopy } from "../../i18n";
import type { Dictionary } from "../../bridge/generated/Dictionary";
import type { DictionaryConflict } from "../../bridge/generated/DictionaryConflict";
import type { DictionaryRequest } from "../../bridge/generated/DictionaryRequest";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { MultiSelectOption } from "../../design/MultiSelect";

// Which dictionaries a meeting uses, and the orders that change them, for a
// hub that takes several (its meeting documents carry `dictionary_ids`) and
// for one up to 0.10.1, which takes one (`contracts/hub-v1.md`, «Meeting
// dictionaries»).

/** The meeting's dictionaries in the hub's order; `null` while not read from the hub. */
export function meetingDictionaries(note: OpenNoteSnapshot): string[] | null {
  const dictionary = note.dictionary;
  if (dictionary === undefined) return null;
  const single = dictionary.dictionary_id;
  return dictionary.dictionary_ids ?? (single === null ? [] : [single]);
}

/** Whether the hub takes lists: only its documents say `dictionary_ids`. */
export function takesLists(note: OpenNoteSnapshot): boolean {
  return note.dictionary?.dictionary_ids !== undefined;
}

/**
 * A choice made in a multi-select, as the hub can take it: any set from a
 * hub that takes lists, at most the last one picked from an older one.
 */
export function chosen(note: OpenNoteSnapshot, before: string[], after: string[]): string[] {
  if (takesLists(note) || after.length <= 1) return after;
  return after.filter((id) => !before.includes(id)).slice(-1);
}

/** «Corregir y añadir»: the correction into every chosen dictionary at once. */
export function correctionOrder(
  note: OpenNoteSnapshot,
  ids: string[],
  wrong: string,
  correct: string,
  name: string | null,
): DictionaryRequest {
  const base = {
    action: "correct" as const,
    recording_id: note.recording_id,
    wrong,
    correct,
    name,
  };
  return takesLists(note) ? { ...base, dictionary_ids: ids } : { ...base, dictionary_id: ids[0] };
}

/** «Deshacer»: the correction leaves every dictionary it went into. */
export function undoOrder(ids: string[], wrong: string, correct: string): DictionaryRequest {
  return { action: "remove_correction", dictionary_ids: ids, wrong, correct };
}

/** What the daemon files an order's answers under (`DictionaryRequest::target`). */
export function orderTarget(order: DictionaryRequest): string {
  switch (order.action) {
    case "correct":
      return order.dictionary_ids
        ? `dictionaries:${order.dictionary_ids.join(",")}`
        : `dictionary:${order.dictionary_id ?? ""}`;
    case "remove_correction":
      return `dictionaries:${order.dictionary_ids.join(",")}`;
    case "select":
      return `dictionary_meeting:${order.recording_id}`;
    case "save":
    case "remove":
      return `dictionary:${order.dictionary_id}`;
  }
}

/** Whether the order is on its way, or waits for its turn. */
export function pending(library: NoteLibrarySnapshot | null | undefined, target: string): boolean {
  return (library?.dictionary_pending ?? []).includes(target);
}

/** Why the hub did not take the order, in words, with its numbers. */
export function refusalOf(
  library: NoteLibrarySnapshot | null | undefined,
  target: string,
): string | undefined {
  const failure = (library?.dictionary_failures ?? []).find((one) => one.target === target);
  return failure && getCopy().dictionary.refused(failure.code, failure.detail, failure.numbers);
}

/** What the hub warned of when it took the order. */
export function conflictsOf(
  library: NoteLibrarySnapshot | null | undefined,
  target: string,
): DictionaryConflict[] {
  return (
    (library?.dictionary_conflicts ?? []).find((one) => one.target === target)?.conflicts ?? []
  );
}

function dictionaries(library: NoteLibrarySnapshot | null | undefined): Dictionary[] {
  return (library?.dictionary_items ?? []).map((item) => item.dictionary);
}

/** A dictionary's name, or its id while the list is not read. */
export function nameOf(library: NoteLibrarySnapshot | null | undefined, id: string): string {
  return dictionaries(library).find((dictionary) => dictionary.id === id)?.name ?? id;
}

/** Whether each of these dictionaries reads `wrong` as `correct`. */
export function holds(
  library: NoteLibrarySnapshot | null | undefined,
  ids: string[],
  wrong: string,
  correct: string,
): boolean {
  const heard = wrong.toLocaleLowerCase("es");
  return ids.every(
    (id) =>
      dictionaries(library)
        .find((dictionary) => dictionary.id === id)
        ?.terms.some(
          (term) =>
            term.correct === correct &&
            term.wrong.some((form) => form.toLocaleLowerCase("es") === heard),
        ) === true,
  );
}

/** Whether none of these dictionaries reads `wrong` as `correct` any more. */
export function released(
  library: NoteLibrarySnapshot | null | undefined,
  ids: string[],
  wrong: string,
  correct: string,
): boolean {
  return ids.every((id) => !holds(library, [id], wrong, correct));
}

/** The rows of a dictionary multi-select: what uses each, its words, and «esta reunión». */
export function dictionaryOptions(
  library: NoteLibrarySnapshot | null | undefined,
  meeting: string[],
): MultiSelectOption[] {
  return dictionaries(library).map((dictionary) => ({
    id: dictionary.id,
    name: dictionary.name,
    detail: getCopy().notes.dictionaryDetail(
      dictionary.series.map((series) => series.title),
      dictionary.terms.length,
    ),
    tag: meeting.includes(dictionary.id) ? getCopy().notes.thisMeeting : undefined,
  }));
}
