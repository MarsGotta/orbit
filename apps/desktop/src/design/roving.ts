import type { KeyboardEvent } from "react";

/**
 * Arrow keys over a row of choices (a radio group, a tab list): the next or
 * previous one, wrapping around, is chosen and takes the focus.
 */
export function moveWithArrows<T>(
  event: KeyboardEvent<HTMLElement>,
  values: readonly T[],
  current: T,
  choose: (value: T) => void,
): void {
  const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
  if (step === undefined) return;
  event.preventDefault();
  const index = (values.indexOf(current) + step + values.length) % values.length;
  const next = values[index];
  if (next === undefined) return;
  choose(next);
  const siblings = event.currentTarget.parentElement?.children;
  const target = siblings?.item(index);
  if (target instanceof HTMLElement) target.focus();
}
