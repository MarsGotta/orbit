import { useCopy } from "../../i18n/CopyProvider";
import { Button } from "../../design/Button";
import type { Distillation } from "../../bridge/generated/Distillation";
import type { DistillationAction } from "../../bridge/generated/DistillationAction";
import { countChanges, countNotes, noteName } from "./distillation-format";
import { noteRoute } from "./route";
import styles from "./Distillation.module.css";
export function DistillationStatus({
  proposal,
  disabled,
  action,
}: {
  proposal: Distillation;
  disabled: boolean;
  action: (action: DistillationAction) => void;
}) {
  const copy = useCopy().distillation;
  const status = proposal.status;
  if (status === "queued") return <p role="status">{copy.queued}</p>;
  if (status === "preparing") return <p role="status">{copy.preparing}</p>;
  if (status === "discarded") return <p role="status">{copy.discarded}</p>;
  if (status === "applied" || status === "undone")
    return <Result proposal={proposal} disabled={disabled} action={action} />;
  if (status === "failed" || status === "stale" || status === "recovery_required")
    return (
      <div className={styles.warning} role="alert">
        <p>
          {status === "stale"
            ? copy.stale(proposal.changed_paths.map((path) => noteName(proposal, path)).join(", "))
            : status === "recovery_required"
              ? copy.recovery
              : proposal.new_source !== null && proposal.changes.length > 0
                ? copy.incomplete
                : copy.failed}
        </p>
        {proposal.error !== null && (
          <p>
            {copy.failure(
              String(proposal.error.code ?? ""),
              Object.fromEntries(
                Object.entries(proposal.error).filter(([, v]) => typeof v === "number"),
              ),
            )}
          </p>
        )}
        {status === "recovery_required" ? (
          <Button disabled={disabled} onClick={() => action("undo")}>
            {copy.undo}
          </Button>
        ) : (
          proposal.can_prepare_again && (
            <Button disabled={disabled} onClick={() => action("prepare")}>
              {status === "failed" ? copy.retry : copy.prepareAgain}
            </Button>
          )
        )}
      </div>
    );
  return null;
}

function Result({
  proposal,
  disabled,
  action,
}: {
  proposal: Distillation;
  disabled: boolean;
  action: (action: DistillationAction) => void;
}) {
  const copy = useCopy().distillation;
  const status = proposal.status;
  return (
    <div className={styles.result} role="status">
      <p>
        {status === "applied"
          ? copy.applied(countChanges(proposal.changes), countNotes(proposal))
          : copy.undone}
      </p>
      {proposal.skipped.map((skip) => (
        <p key={skip.path}>
          {copy.skipped(
            noteName(proposal, skip.path),
            skip.code === "content_changed" ? copy.changedAfter : copy.outcomeUnknown,
          )}
        </p>
      ))}
      {status === "applied" && (
        <Button disabled={disabled} onClick={() => action("undo")}>
          {copy.undo}
        </Button>
      )}
      {status === "applied" &&
        proposal.changes
          .filter((change) => change.include)
          .slice(0, 1)
          .map((change) => (
            <a key={change.id} href={noteRoute(change.path)}>
              {copy.seeNote(change.title)}
            </a>
          ))}
      <a href={noteRoute("log.md")}>{copy.log}</a>
    </div>
  );
}
