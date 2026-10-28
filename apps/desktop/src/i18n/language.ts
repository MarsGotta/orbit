import type { LanguageChoice } from "../bridge/generated/LanguageChoice";
export type { LanguageChoice };
export type Language = "es" | "en";
export type Locale = "es-ES" | "en-US" | "en-GB";

export function systemLanguages(): readonly string[] {
  return typeof navigator === "undefined" ? [] : navigator.languages;
}

export function resolveLanguage(choice: LanguageChoice, preferred: readonly string[]) {
  const supported = preferred.find((tag) => /^(es|en)(-|_|$)/i.test(tag));
  const language: Language =
    choice === "system" ? (supported?.toLowerCase().startsWith("es") ? "es" : "en") : choice;
  const regional = preferred.find(
    (tag) => tag.toLowerCase().startsWith(`${language}-`) || tag.toLowerCase() === language,
  );
  const locale: Locale =
    language === "es" ? "es-ES" : /^en[-_]gb(?:-|_|$)/i.test(regional ?? "") ? "en-GB" : "en-US";
  return { language, locale };
}
