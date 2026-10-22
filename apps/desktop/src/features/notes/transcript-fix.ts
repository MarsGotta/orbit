import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import { conflictsOf } from "./dictionary-choice";
import type { Correction } from "./WordMenu";
import { corrected, fixedPlaces, type Span } from "./word-forms";

/**
 * A correction the hub took, laid over the transcript until the note is
 * left: each turn that said the word reads it corrected, the corrected words
 * marked. A turn whose text is neither the one corrected nor its corrected
 * form (transcribed again since) shows the hub's text as it is.
 */
export interface Fix {
  correction: Correction;
  before: Map<number, string>;
  after: Map<number, string>;
  places: Map<number, Span[]>;
}

export function fixOf(
  library: NoteLibrarySnapshot | null | undefined,
  correction: Correction,
): Fix {
  // Another dictionary of the meeting may win: the meeting then says its word.
  const won = conflictsOf(library, correction.target).find(
    (conflict) => conflict.applied_dictionary_id === conflict.other_dictionary_id,
  );
  const shown = won?.other_correct ?? correction.correct;
  const fix: Fix = { correction, before: new Map(), after: new Map(), places: new Map() };
  for (const place of fixedPlaces(correction.before, correction.wrong, shown)) {
    const turn = correction.before.find((one) => one.index === place.turn);
    if (!turn) continue;
    fix.before.set(turn.index, turn.text);
    fix.after.set(turn.index, corrected(turn.text, correction.wrong, shown));
    fix.places.set(turn.index, [...(fix.places.get(turn.index) ?? []), place]);
  }
  return fix;
}

/** The turn's text with the correction in, or the hub's when it is another text. */
export function shownText(turn: NoteTurnSnapshot, fix: Fix | null): string {
  const after = fix?.after.get(turn.index);
  const ours = turn.text === after || turn.text === fix?.before.get(turn.index);
  return after !== undefined && ours ? after : turn.text;
}
