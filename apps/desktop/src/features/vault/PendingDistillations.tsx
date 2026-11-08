import { proposalLead, pendingHeading } from "./distillation-presentation";
import pendingStyles from "./PendingDistillations.module.css";
import type { DistillationList } from "./use-distillations";
import { useCopy } from "orbit-ui/i18n/CopyProvider";
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import { Button } from "orbit-ui/design/Button";
import { countChanges, pending, proposalRoute } from "./distillation-format";
import column from "./VaultColumn.module.css";
import styles from "./VaultReading.module.css";
export function PendingLinks({ entries, page, nextPage, setPage }: DistillationList) {
  const copy = useCopy().distillation;
  const proposals = entries.filter(pending);
  if (proposals.length === 0 && page === 1 && nextPage === null) return null;
  return (
    <section>
      <h3 className={column.caption}>{copy.pending}</h3>
      {proposals.map((proposal) => (
        <a
          className={pendingStyles.item}
          href={proposalRoute(proposal.id)}
          key={proposal.id}
          aria-current={window.location.hash === proposalRoute(proposal.id) ? "page" : undefined}
        >
          <span className={pendingStyles.dot} aria-hidden="true">
            ●
          </span>{" "}
          <span>{proposal.title || copy.untitledMeeting}</span>
          <small>{copy.changes(countChanges(proposal.changes))}</small>
        </a>
      ))}
      <div className={styles.actions}>
        {page > 1 && <Button onClick={() => setPage(page - 1)}>{copy.previousProposals}</Button>}
        {nextPage !== null && nextPage > page && (
          <Button onClick={() => setPage(nextPage)}>{copy.nextProposals}</Button>
        )}
      </div>
    </section>
  );
}
export function PendingDistillations({
  entries,
  error,
  loading,
  reload,
}: {
  entries: Distillation[];
  error: string | null;
  loading: boolean;
  reload: () => void;
}) {
  const copy = useCopy().distillation;
  return (
    <section>
      {error !== null && (
        <p role="alert">
          {error} <Button onClick={reload}>{copy.reload}</Button>
        </p>
      )}
      {loading && <p role="status">{copy.reading}</p>}
      {entries.filter(pending).map((proposal) => (
        <PendingCard key={proposal.id} proposal={proposal} />
      ))}
    </section>
  );
}

function PendingCard({ proposal }: { proposal: Distillation }) {
  const copy = useCopy().distillation;
  const undecided = proposal.conflicts.filter((conflict) => conflict.choice === null).length;
  return (
    <article className={pendingStyles.card}>
      <span className={pendingStyles.avatar} aria-hidden="true">
        {copy.author.charAt(0)}
      </span>
      <div className={pendingStyles.body}>
        <h2>{pendingHeading(proposal, copy)}</h2>
        <p>{proposalLead(proposal, copy)}</p>
        {undecided > 0 && <p className={pendingStyles.warning}>{copy.conflictCount(undecided)}</p>}
        <div className={pendingStyles.chips}>
          {proposal.changes.map((change) => (
            <span key={change.id}>
              {change.title} · {copy.changes(countChanges([{ ...change, include: true }]))}
            </span>
          ))}
        </div>
      </div>
      <a className={pendingStyles.cta} href={proposalRoute(proposal.id)}>
        {copy.review}
      </a>
    </article>
  );
}
