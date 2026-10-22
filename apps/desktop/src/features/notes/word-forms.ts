// A word of the transcript as the hub's dictionaries see it
// (`server/orbit_hub/meetings/dictionaries.py`, `substitute`): a run between
// two characters that are not letters, digits, marks or «_», matched without
// regard to case. Counting and marking a correction here follow that rule, so
// «Sale 3 veces» counts the places the hub will change.

const WORD = "[\\p{L}\\p{N}\\p{M}_]";
const WORD_CHAR = new RegExp(WORD, "u");

/** Where a word starts and ends in a turn's text, `end` excluded. */
export interface Span {
  start: number;
  end: number;
}

/** The word that holds the character at `offset`, or none on a space or a sign. */
export function wordAt(text: string, offset: number): Span | null {
  if (offset < 0 || offset >= text.length || !WORD_CHAR.test(text.charAt(offset))) return null;
  let start = offset;
  while (start > 0 && WORD_CHAR.test(text.charAt(start - 1))) start -= 1;
  let end = offset + 1;
  while (end < text.length && WORD_CHAR.test(text.charAt(end))) end += 1;
  return { start, end };
}

function escaped(form: string): string {
  return form.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Every place `form` is said in `text`, whole, in any case. */
export function occurrences(text: string, form: string): Span[] {
  const wanted = form.trim();
  if (wanted === "") return [];
  const pattern = new RegExp(`(?<!${WORD})${escaped(wanted)}(?!${WORD})`, "giu");
  return Array.from(text.matchAll(pattern), (match) => ({
    start: match.index,
    end: match.index + match[0].length,
  }));
}

/** One place a correction changed: the turn, and where the corrected word sits. */
export interface Fixed extends Span {
  turn: number;
}

/**
 * Where the corrected words sit once `wrong` reads `correct` in each turn:
 * each later place in a turn moves by what the earlier ones grew or shrank.
 */
export function fixedPlaces(
  turns: readonly { index: number; text: string }[],
  wrong: string,
  correct: string,
): Fixed[] {
  const growth = correct.length;
  return turns.flatMap((turn) =>
    occurrences(turn.text, wrong).map((place, order) => {
      const start = place.start + order * (growth - (place.end - place.start));
      return { turn: turn.index, start, end: start + growth };
    }),
  );
}

/**
 * The turn's text as it reads once the correction is in: the hub's own read
 * when it already brings it, otherwise each place of `wrong` written as
 * `correct` until that read arrives.
 */
export function corrected(text: string, wrong: string, correct: string): string {
  let out = "";
  let at = 0;
  for (const place of occurrences(text, wrong)) {
    out += text.slice(at, place.start) + correct;
    at = place.end;
  }
  return out + text.slice(at);
}
