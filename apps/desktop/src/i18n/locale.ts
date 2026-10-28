import type { Locale } from "./language";
let current: Locale = "en-US";
export function setLocale(locale: Locale) {
  current = locale;
}
export function getLocale(): Locale {
  return current;
}

/** Lazy formatters let module-level helpers follow live language changes. */
export function dateTime(options?: Intl.DateTimeFormatOptions) {
  return {
    format: (date: number | Date) => new Intl.DateTimeFormat(current, options).format(date),
    formatRange: (start: number, end: number) =>
      new Intl.DateTimeFormat(current, options).formatRange(start, end),
  };
}
export function numberFormat(options?: Intl.NumberFormatOptions) {
  return { format: (value: number) => new Intl.NumberFormat(current, options).format(value) };
}
export function listFormat(options?: Intl.ListFormatOptions) {
  return {
    format: (values: Iterable<string>) => new Intl.ListFormat(current, options).format(values),
  };
}

/** Numeric facts in open wire records keep their locale without coercing
 * unknown strings or codes into numbers. */
export function formatQuantity(value: unknown): string {
  return typeof value === "number" && Number.isFinite(value)
    ? numberFormat().format(value)
    : String(value);
}
