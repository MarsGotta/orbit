import { setLocale } from "./locale";
export { getLocale } from "./locale";
import { english } from "./en";
import { spanish } from "./es";
import type { Catalogue } from "./catalogue";
import { resolveLanguage, systemLanguages, type LanguageChoice } from "./language";
export { resolveLanguage } from "./language";
export type { Catalogue } from "./catalogue";

let current = { ...resolveLanguage("system", systemLanguages()), name: "Deimos" };
setLocale(current.locale);
let copy: Catalogue = current.language === "es" ? spanish() : english();

/** The provider publishes the same language to code outside React, including
 * event handlers and formatting functions. Each webview owns its own module. */
export function configureCopy(choice: LanguageChoice, languages: readonly string[], name: string) {
  const resolved = resolveLanguage(choice, languages);
  if (
    current.language !== resolved.language ||
    current.locale !== resolved.locale ||
    current.name !== name
  ) {
    copy = resolved.language === "es" ? spanish(name) : english(name);
  }
  current = { ...resolved, name };
  setLocale(resolved.locale);
  return copy;
}

export function getCopy(name?: string): Catalogue {
  if (name === undefined || name === current.name) return copy;
  return current.language === "es" ? spanish(name) : english(name);
}

export type { NoticeAction, NoticeData } from "./es/notices";
export type { FailedNoticeCode } from "./es/failures";

export { lookupCopy } from "./lookup";
