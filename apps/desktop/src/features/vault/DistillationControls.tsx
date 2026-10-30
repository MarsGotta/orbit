import { useCopy } from "../../i18n/CopyProvider";
import { Button } from "../../design/Button";
import type { Distillation } from "../../bridge/generated/Distillation";
import type { useDistillation } from "./use-distillation";
import { countChanges } from "./distillation-format";
import styles from "./Distillation.module.css";
type Model = ReturnType<typeof useDistillation>;
export function DistillationControls({
  proposal,
  model,
  editing,
}: {
  proposal: Distillation;
  model: Model;
  editing: boolean;
}) {
  const copy = useCopy().distillation;
  const missing = proposal.conflicts.filter((conflict) => conflict.choice === null);
  const n = countChanges(proposal.changes);
  const label = applyLabel(proposal, model, editing, copy);
  return (
    <div className={styles.actions}>
      {["queued", "proposed", "failed", "stale"].includes(proposal.status) && (
        <Button
          disabled={model.busy || editing}
          onClick={() =>
            void model.mutate({ call: "distillation_action", id: proposal.id, action: "discard" })
          }
          variant="ghost"
        >
          {copy.discard}
        </Button>
      )}
      {proposal.status === "proposed" && (
        <Button
          disabled={model.busy || editing || missing.length > 0 || n === 0}
          onClick={() =>
            void model.mutate({ call: "distillation_action", id: proposal.id, action: "apply" })
          }
          variant="accent"
        >
          {label}
        </Button>
      )}
      {proposal.status === "applying" && (
        <Button disabled variant="accent">
          {copy.applying}
        </Button>
      )}
    </div>
  );
}

function applyLabel(
  proposal: Distillation,
  model: Model,
  editing: boolean,
  copy: ReturnType<typeof useCopy>["distillation"],
) {
  const missing = proposal.conflicts.filter((conflict) => conflict.choice === null);
  const n = countChanges(proposal.changes);
  if (model.activity === "apply") return copy.applying;
  if (model.busy) return copy.saving;
  if (editing) return copy.finishEditing;
  if (missing.length > 1) return copy.decisionsMissing(missing.length);
  const conflict = missing[0];
  if (conflict !== undefined) {
    const field =
      typeof conflict.incoming === "string" && /^\d{4}-\d{2}-\d{2}$/.test(conflict.incoming)
        ? copy.dateDecision
        : conflict.property;
    return copy.decideToApply(field);
  }
  return n === 0 ? copy.noSelection : copy.apply(n);
}
