import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import type { DistillationChange } from "orbit-ui/bridge/generated/DistillationChange";
export function countChanges(changes: DistillationChange[]) {
  return changes
    .filter((change) => change.include)
    .reduce((n, change) => n + change.properties.length + change.lines.length, 0);
}
export function countNotes(proposal: Distillation) {
  return proposal.changes.filter((change) => change.include).length;
}
export function pending(proposal: Distillation) {
  return ["proposed", "failed", "stale", "recovery_required"].includes(proposal.status);
}
export function valueText(value: unknown): string {
  if (value === null || value === undefined) return "—";
  return typeof value === "string" ? value : JSON.stringify(value);
}
export function quoteTime(ms: number) {
  const seconds = Math.floor(ms / 1000);
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}
export function proposalRoute(id: string) {
  return `#/vault/distill/${encodeURIComponent(id)}`;
}
export function noteName(proposal: Distillation, path: string) {
  return (
    proposal.changes.find((change) => change.path === path)?.title ??
    path.split("/").pop()?.replace(/\.md$/, "").replaceAll("-", " ") ??
    path
  );
}
