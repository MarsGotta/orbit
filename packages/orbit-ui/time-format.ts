/** Whole minutes and seconds; recordings may run beyond an hour. */
export function mmss(ms: number, padMinutes = true): string {
  const seconds = Math.max(0, Math.floor(ms / 1_000));
  return `${String(Math.floor(seconds / 60)).padStart(padMinutes ? 2 : 1, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}
