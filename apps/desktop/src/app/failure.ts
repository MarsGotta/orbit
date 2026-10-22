import type { AppCommand } from "../bridge/generated";
import type { Report } from "../bridge/port";
import type { Refusal } from "../bridge/refusal";
import { failures } from "../i18n/es/failures";

/**
 * What a failure offers besides its words: the code of its op to share,
 * the title when its words are the generic ones («No se pudo …»), and the
 * order «Reintentar» sends again, only when the refusal says repeating it
 * is safe.
 */
export interface Failure {
  code: string;
  title?: string;
  retry?: AppCommand;
}

/** The code a failure shows: the first six characters of its op, in capitals (`Op::short`). */
export function failureCode(op: string): string {
  return op.slice(0, 6).toUpperCase();
}

/**
 * The message of a failure the person cannot fix: what could not be done,
 * what to do, and the code of its op to share with whoever runs the hub.
 */
export function genericFailure(op: string, what: string = failures.failure.fallback): string {
  return `${failures.failure.title(what)}. ${failures.failure.help(failureCode(op))}`;
}

/**
 * Whether a refused order may be sent again as it was: only when the
 * refusal says so (`after_fix`, `after_ms`). `check_outcome` means it may
 * already have been done, and `never` that repeating it cannot help.
 */
export function repeatable(refusal: Refusal): boolean {
  const { retry } = refusal;
  if (typeof retry !== "object" || retry === null || !("kind" in retry)) return false;
  return retry.kind === "after_fix" || retry.kind === "after_ms";
}

/** A failure as the interceptor presents it. */
export interface Failed {
  op: string;
  class: "failed";
  text: string;
  failure: Failure;
  report: Report | null;
}

/** The generic failure of an op: «No se pudo {what}», what to do and its code. */
export function failure(op: string, report: Report | null, what?: string): Failed {
  const shown = what ?? failures.failure.fallback;
  return {
    op,
    class: "failed",
    text: genericFailure(op, shown),
    failure: { code: failureCode(op), title: failures.failure.title(shown) },
    report,
  };
}

/** A failure in words of its own: no generic title, the same code. */
export function worded(result: Failed, text: string | undefined): Failed {
  if (text === undefined) return result;
  return { ...result, text, failure: { code: failureCode(result.op) } };
}
