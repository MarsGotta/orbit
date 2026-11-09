import { getCopy } from "../../i18n";
import { actorName } from "./provenance";
export function logAction(action: string) {
  const copy = getCopy().distillation;
  return (
    (
      {
        create: copy.logCreate,
        update: copy.logUpdate,
        delete: copy.logDelete,
        apply: copy.logApply,
      } as Record<string, string>
    )[action] ?? action
  );
}
export function logDetail(detail: string) {
  const copy = getCopy().distillation;
  return detail === "Meeting distillation"
    ? copy.logDistillation
    : detail === "Undo meeting distillation"
      ? copy.logUndo
      : detail;
}
/** Format only the ledger's structured rows. Handwritten prose and raw Markdown stay intact. */
export function logBody(body: string, owner: string) {
  return body
    .split("\n")
    .map((line) => {
      const row =
        /^(- \d{2}:\d{2}) · (create|update|delete) · (\[.*\]\([^)]+\)) · (\S+) · (.*)$/.exec(line);
      if (row === null) return line;
      const [, clock = "", action = "", link = "", by = "", detail = ""] = row;
      return [clock, logAction(action), link, actorName(by, owner), logDetail(detail)].join(" · ");
    })
    .join("\n");
}
