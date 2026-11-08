/** Invented public hub proposal; no connector, provider or device effects. */
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import type { VaultCall } from "orbit-ui/bridge/generated/VaultCall";
import type { VaultOutcome } from "orbit-ui/bridge/generated/VaultOutcome";
import fixture from "orbit-ui/features/vault/__fixtures__/distillation.json";
import { vaultAnswer } from "../vault";
export function distillationPort(status: Distillation["status"] = "proposed", holdApply = false) {
  let current = { ...(structuredClone(fixture) as Distillation), status };
  if (status === "applied")
    current.conflicts.forEach((conflict) => {
      conflict.choice = "incoming";
    });
  return (call: VaultCall): VaultOutcome | Promise<VaultOutcome> => {
    if (call.call === "list_distillations")
      return {
        outcome: "distillations",
        page: { entries: current.status === "applied" ? [] : [current], next_page: null },
      };
    if (call.call === "review_distillation") {
      current = structuredClone(current);
      current.revision++;
      for (const conflict of current.conflicts)
        conflict.choice = call.review.decisions?.[conflict.id] ?? conflict.choice;
      for (const change of current.changes) {
        const edit = call.review.changes?.[change.id];
        if (edit === undefined) continue;
        change.include = edit.include ?? change.include;
        for (const property of change.properties)
          if (edit.properties !== undefined && property.key in edit.properties)
            property.after = edit.properties[property.key];
        change.lines.forEach((line, i) => {
          line.after = edit.lines?.[String(i)] ?? line.after;
        });
      }
    }
    if (call.call === "distillation_action") {
      if (call.action === "apply" && holdApply) return new Promise(() => undefined);
      current = {
        ...current,
        status:
          call.action === "apply"
            ? "applied"
            : call.action === "undo"
              ? "undone"
              : call.action === "discard"
                ? "discarded"
                : "queued",
      };
    }
    if (["distillation", "distillation_action", "review_distillation"].includes(call.call))
      return { outcome: "distillation", proposal: current };
    const answer = vaultAnswer(call, false);
    return answer.outcome === "catalog" && current.status === "applied"
      ? {
          ...answer,
          catalog: { ...answer.catalog, sources_count: answer.catalog.sources_count + 1 },
        }
      : answer;
  };
}
