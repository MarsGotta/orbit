import { proposalLead } from "./distillation-presentation";
import { DistillationControls } from "./DistillationControls";
import { useState } from "react";
import { useCopy } from "orbit-ui/i18n/CopyProvider";
import { Breadcrumbs, TopBarLeading } from "../../app/shell-slots";
import { Button } from "orbit-ui/design/Button";
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import { type VaultPort } from "./api";
import { useDistillation } from "./use-distillation";
import { DistillationColumn } from "./DistillationColumn";
import { DistillationConflict } from "./DistillationConflict";
import { DistillationNote } from "./DistillationNote";
import { DistillationStatus } from "./DistillationStatus";
import styles from "./Distillation.module.css";
type Model = ReturnType<typeof useDistillation>;
function Proposal({ proposal, model }: { proposal: Distillation; model: Model }) {
  const copy = useCopy().distillation;
  const [choice, select] = useState("");
  const selected =
    choice !== "" ? choice : (proposal.conflicts[0]?.id ?? proposal.changes[0]?.id ?? "");
  const [editing, setEditing] = useState(false);
  const disabled =
    model.busy || ["applying", "undoing", "queued", "preparing"].includes(proposal.status);
  return (
    <section className={styles.view}>
      <TopBarLeading>
        <div className={styles.toolbar}>
          <Breadcrumbs
            trail={[
              { label: copy.pending, href: "#/vault" },
              { label: proposal.title || copy.untitledMeeting },
            ]}
          />
          <DistillationControls {...{ proposal, model, editing }} />
        </div>
      </TopBarLeading>
      <ProposalHeader proposal={proposal} />
      {model.error !== null && (
        <p role="alert">
          {model.error}{" "}
          <Button disabled={model.busy || editing} onClick={model.reload}>
            {copy.reload}
          </Button>
        </p>
      )}
      <DistillationStatus
        proposal={proposal}
        disabled={disabled}
        action={(action) =>
          void model.mutate({ call: "distillation_action", id: proposal.id, action })
        }
      />
      <ReviewLayout {...{ proposal, selected, select, disabled, editing, setEditing, model }} />
    </section>
  );
}
export function DistillationView({ port, id }: { port?: VaultPort; id: string }) {
  const copy = useCopy().distillation;
  const model = useDistillation(port, id);
  if (model.proposal === null)
    return (
      <div className={styles.detail}>
        {model.error === null ? (
          <p role="status">{copy.reading}</p>
        ) : (
          <p role="alert">
            {model.error} <Button onClick={model.reload}>{copy.reload}</Button>
          </p>
        )}
      </div>
    );
  return <Proposal key={model.proposal.id} proposal={model.proposal} model={model} />;
}

function ProposalHeader({ proposal }: { proposal: Distillation }) {
  const copy = useCopy().distillation;
  return (
    <header className={styles.header}>
      <div>
        <h1>{proposal.title === "" ? copy.titleFallback : copy.title(proposal.title)}</h1>
        <p>{proposalLead(proposal, copy)}</p>
      </div>
      <a href={`#/notes/${encodeURIComponent(proposal.meeting_id)}`}>{copy.openMeeting}</a>
    </header>
  );
}

function ReviewLayout({
  proposal,
  selected,
  select,
  disabled,
  editing,
  setEditing,
  model,
}: {
  proposal: Distillation;
  selected: string;
  select: (value: string) => void;
  disabled: boolean;
  editing: boolean;
  setEditing: (value: boolean) => void;
  model: Model;
}) {
  if (proposal.changes.length === 0 && proposal.conflicts.length === 0) return null;
  const conflict = proposal.conflicts.find((c) => c.id === selected);
  const change = proposal.changes.find((c) => c.id === selected);
  return (
    <div className={styles.layout}>
      <DistillationColumn
        proposal={proposal}
        selected={selected}
        select={select}
        disabled={disabled || editing}
        review={(review) =>
          void model.mutate({ call: "review_distillation", id: proposal.id, review })
        }
      />
      {conflict !== undefined && (
        <DistillationConflict
          proposal={proposal}
          conflict={conflict}
          disabled={disabled}
          choose={(choice) =>
            void model.mutate({
              call: "review_distillation",
              id: proposal.id,
              review: { revision: proposal.revision, decisions: { [conflict.id]: choice } },
            })
          }
        />
      )}
      {change !== undefined && (
        <DistillationNote
          key={change.id}
          proposal={proposal}
          change={change}
          disabled={disabled}
          onEditing={setEditing}
          review={(review) =>
            model.mutate({ call: "review_distillation", id: proposal.id, review })
          }
        />
      )}
    </div>
  );
}
