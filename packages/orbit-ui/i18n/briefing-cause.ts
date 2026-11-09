import type { Refusal } from "../bridge/refusal";

export type CauseFields = Readonly<Record<string, unknown>>;
export function causeFields(value: unknown): CauseFields {
  return typeof value === "object" && value !== null ? (value as CauseFields) : {};
}

/** Briefing preserves source codes and serialized refusal records until display. */
export function decodeBriefingCause(value: string) {
  const encoded = value.startsWith("agenda_request_failed:")
    ? value.slice("agenda_request_failed:".length)
    : value;
  let fields: CauseFields = {};
  try {
    fields = causeFields(JSON.parse(encoded));
  } catch {
    // Plain codes are used for states with no additional facts.
  }
  const code = typeof fields.code === "string" ? fields.code : value;
  const cause = causeFields(fields.cause);
  if (code === "internal" && cause.kind === "app_operation" && typeof cause.code === "string") {
    let facts: CauseFields = {};
    try {
      facts = typeof cause.error === "string" ? causeFields(JSON.parse(cause.error)) : {};
    } catch {
      // Unstructured diagnostics are never interface copy.
    }
    return { code: cause.code, facts, refusal: undefined };
  }
  return {
    code,
    facts: fields,
    refusal: typeof fields.code === "string" ? (fields as Refusal) : undefined,
  };
}

/** Only numeric wire facts reach count and date formatters. */
export function hasNumbers<K extends string>(
  facts: CauseFields,
  ...keys: K[]
): facts is CauseFields & Record<K, number> {
  return keys.every((key) => typeof facts[key] === "number" && Number.isFinite(facts[key]));
}
