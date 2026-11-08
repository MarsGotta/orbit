import { useState } from "react";
import { useCopy } from "orbit-ui/i18n/CopyProvider";
import { Button } from "orbit-ui/design/Button";
import { useDistillations } from "../vault/use-distillations";
import { useVaultConfiguration } from "../vault/use-vault-read";
import { countChanges, countNotes, proposalRoute } from "../vault/distillation-format";
import { noteRoute } from "../vault/route";
import { message, request, type VaultPort } from "../vault/api";
import styles from "./MeetingDistillation.module.css";
export function MeetingDistillation({ port, meetingId }: { port?: VaultPort; meetingId: string }) {
  const copy = useCopy().distillation;
  const { configuration } = useVaultConfiguration(port);
  const configured = configuration?.configured === true;
  const { entries, error, reload } = useDistillations(configured ? port : undefined, meetingId);
  const { busy, failure, prepare } = usePrepareMeeting(port, meetingId, reload);
  const proposal = entries[0];
  if (port === undefined || !configured) return null;
  const pill = proposal?.status === "proposed" || proposal?.status === "applied";
  return (
    <div className={styles.wrap}>
      {pill && (
        <a
          className={styles.pill}
          data-applied={proposal.status === "applied"}
          href={proposal.status === "applied" ? noteRoute("log.md") : proposalRoute(proposal.id)}
          title={
            proposal.status === "applied"
              ? copy.appliedTip(countChanges(proposal.changes), countNotes(proposal))
              : copy.reviewTip(countChanges(proposal.changes), countNotes(proposal))
          }
        >
          <span aria-hidden="true">●</span>
          {proposal.status === "applied"
            ? copy.inNotes(countChanges(proposal.changes))
            : copy.reviewPill(countChanges(proposal.changes))}
        </a>
      )}
      {proposal !== undefined && !pill && !["discarded", "undone"].includes(proposal.status) && (
        <a href={proposalRoute(proposal.id)}>{proposalState(proposal.status, copy)}</a>
      )}
      {!pill &&
        (proposal === undefined || ["discarded", "undone"].includes(proposal.status)) &&
        configuration?.configured === true &&
        configuration.maintenance_enabled && (
          <Button disabled={busy} onClick={() => void prepare()}>
            {busy ? copy.preparing : copy.prepareMeeting}
          </Button>
        )}
      {(error !== null || failure !== null) && (
        <p role="alert">
          {failure !== null ? message(failure) : error}{" "}
          <Button onClick={reload}>{copy.reload}</Button>
        </p>
      )}
    </div>
  );
}

function usePrepareMeeting(port: VaultPort | undefined, meetingId: string, reload: () => void) {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const prepare = async () => {
    if (busy) return;
    setBusy(true);
    setFailure(null);
    try {
      const answer = await request(port, { call: "prepare_distillation", meeting_id: meetingId });
      if (answer.outcome === "distillation")
        window.location.hash = proposalRoute(answer.proposal.id);
      reload();
    } catch (error: unknown) {
      setFailure(error);
    } finally {
      setBusy(false);
    }
  };
  return { busy, failure, prepare };
}

function proposalState(status: string, copy: ReturnType<typeof useCopy>["distillation"]) {
  if (status === "queued") return copy.queued;
  if (status === "failed") return copy.failed;
  if (status === "stale") return copy.prepareAgain;
  if (status === "applying") return copy.applying;
  if (status === "undoing") return copy.undoing;
  if (status === "recovery_required") return copy.recoveryTitle;
  return copy.preparing;
}
