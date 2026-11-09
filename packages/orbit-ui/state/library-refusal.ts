import { getCopy } from "../i18n";
import type { NoteLibraryRefusalSnapshot } from "../bridge/generated/NoteLibraryRefusalSnapshot";

/** The hub's refusal of an order, as what the user can do about it. */
export function refusalText(refusal: NoteLibraryRefusalSnapshot): string {
  return getCopy().library.refused(refusal.code, refusal.numbers);
}

/** The refusals of one series' assignment, newest last. */
export function seriesRefusals(
  refusals: NoteLibraryRefusalSnapshot[] | undefined,
  calendarItemId: string,
): NoteLibraryRefusalSnapshot[] {
  return (refusals ?? []).filter(
    (refusal) => refusal.target_kind === "series" && refusal.target_id === calendarItemId,
  );
}
