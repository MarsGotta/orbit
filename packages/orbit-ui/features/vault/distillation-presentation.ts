import type { Catalogue } from "../../i18n/catalogue";
import type { Distillation } from "../../bridge/generated/Distillation";
import { countChanges, countNotes } from "./distillation-format";
type Copy = Catalogue["distillation"];
export function proposalLead(proposal: Distillation, copy: Copy) {
  switch (proposal.status) {
    case "queued":
    case "preparing":
      return copy.preparingLead;
    case "failed":
      return copy.failedLead;
    case "stale":
      return copy.staleLead;
    case "applying":
      return copy.applyingLead;
    case "applied":
      return copy.appliedLead;
    case "discarded":
      return copy.discardedLead;
    case "undoing":
      return copy.undoingLead;
    case "undone":
      return copy.undoneLead;
    case "recovery_required":
      return copy.recoveryLead;
    case "proposed":
      return copy.lead(countChanges(proposal.changes), countNotes(proposal));
  }
}
export function sourceHeading(proposal: Distillation, copy: Copy) {
  if (proposal.status === "discarded") return copy.sourceNotSaved;
  if (proposal.status === "applied" || proposal.status === "undone") return copy.sourceSaved;
  return proposal.status === "proposed" ? copy.source : copy.sourcePlan;
}
export function pendingHeading(proposal: Distillation, copy: Copy) {
  const title = proposal.title || copy.untitledMeeting;
  if (proposal.status === "failed")
    return proposal.new_source !== null && proposal.changes.length > 0
      ? copy.incomplete
      : copy.failed;
  if (proposal.status === "recovery_required") return copy.recoveryTitle;
  return proposal.status === "proposed" ? copy.pendingTitle(title) : title;
}
