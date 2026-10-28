import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
export const dictionary = {
  dictionary: {
    label: "Dictionary",
    name: "Dictionary name",
    inherit: "Use series dictionary",
    create: "Create a dictionary",
    correctText: "Correct text",
    correctForm: "Correct form",
    saveCorrection: "Save correction",
    saving: "Saving to the hub…",
    unread: "The dictionary couldn’t be read from the hub yet.",
    selected: (text: string) => `Selected text: “${text}”`,
    help: "Corrects every occurrence in meetings using this dictionary. Update an existing note by redoing its summary.",
    empty: "No dictionaries yet. Create one when correcting a word in a transcript.",
    emptyTerms: "No words corrected yet.",
    noSeries: "No series always uses this",
    series: (names: string) => `Always used by: ${names}`,
    insteadOf: (wrong: string) => `instead of ${wrong}`,
    rename: "Rename",
    renameLabel: (name: string) => `Rename ${name}`,
    // Above the cards: several per meeting, and how words get in.
    several:
      "A meeting or series can use several dictionaries. Add words by clicking a word in a transcript and choosing “Correct”.",
    words: (count: number) =>
      pluralRules().select(count) === "one" ? "1 word" : `${numberFormat().format(count)} words`,
    seeAll: (count: number) => `View ${numberFormat().format(count)} words`,
    removeEntry: "Remove",
    remove: (term: string) => `Remove ${term}`,
    refused: (code: string, _detail: string, numbers: Record<string, number>) => {
      const causes: Record<string, string> = {
        not_paired: "Orbit isn’t paired with the hub",
        privacy_stopped: "Everything is stopped",
        // The transport's own cause stays in the log.
        hub_unreachable: "Change not saved: no connection to the hub",

        dictionary_full: "The dictionary is full",
        dictionary_text_too_long: "The term or name is too long",
        dictionary_forms_full: "The term has too many incorrect forms",
        dictionaries_full: "Too many dictionaries",
        dictionary_too_large: "The dictionary takes too many bytes",
        dictionaries_too_large: "The dictionary list takes too many bytes",
        dictionary_series_full: "Too many assigned series",
        dictionary_series_missing: "The meeting has no calendar series",
        dictionary_not_found: "The dictionary doesn’t exist",
        dictionary_text_invalid: "The term or name is empty or contains control characters",
      };
      const labels: Record<string, string> = {
        terms: "terms",
        max_terms: "maximum terms",
        forms: "forms",
        max_forms: "maximum forms",
        characters: "characters",
        max_characters: "maximum characters",
        dictionaries: "dictionaries",
        max_dictionaries: "maximum dictionaries",
        series: "series",
        max_series: "maximum series",
        bytes: "bytes",
        max_bytes: "maximum bytes",
        used_bytes: "bytes used",
      };
      const values = Object.entries(numbers)
        .map(([key, value]) => `${labels[key] ?? key}: ${numberFormat().format(value)}`)
        .join(", ");
      return `${(Object.hasOwn(causes, code) ? causes[code] : undefined) ?? `Orbit does not recognise this code: ${code}`}${values ? ` (${values})` : ""}.`;
    },
  },
} as const;
