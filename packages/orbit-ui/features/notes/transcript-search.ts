/** One occurrence of the searched text: in which turn and where. */
export interface Match {
  segment: number;
  start: number;
  end: number;
}

const fold = (text: string) =>
  text
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLocaleLowerCase("es");

/**
 * Every occurrence of `query` in the turns, in reading order. Accents and
 * case do not matter; the positions are those of the original text (NFD
 * folding drops only combining marks, so a letter stays one position).
 */
export function findMatches(texts: string[], query: string): Match[] {
  const needle = fold(query.trim());
  if (needle === "") return [];
  const matches: Match[] = [];
  texts.forEach((text, segment) => {
    const hay = fold(text);
    if (hay.length !== text.length) {
      // A character that folds into several: search the folded text only
      // for presence, and mark the whole turn.
      if (hay.includes(needle)) matches.push({ segment, start: 0, end: text.length });
      return;
    }
    let from = hay.indexOf(needle);
    while (from >= 0) {
      matches.push({ segment, start: from, end: from + needle.length });
      from = hay.indexOf(needle, from + needle.length);
    }
  });
  return matches;
}
