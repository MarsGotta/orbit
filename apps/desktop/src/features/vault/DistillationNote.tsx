import { useCopy } from "orbit-ui/i18n/CopyProvider";
import { Button } from "orbit-ui/design/Button";
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import type { DistillationChange } from "orbit-ui/bridge/generated/DistillationChange";
import type { DistillationReview } from "orbit-ui/bridge/generated/DistillationReview";
import { useDistillationEdit } from "./use-distillation-edit";
import { DistillationQuotes } from "./DistillationQuotes";
import { PropertyDiff, TextDiff } from "./DistillationDiff";
import styles from "./Distillation.module.css";
export function DistillationNote({
  proposal,
  change,
  disabled,
  onEditing,
  review,
}: {
  proposal: Distillation;
  change: DistillationChange;
  disabled: boolean;
  onEditing: (value: boolean) => void;
  review: (review: DistillationReview) => Promise<boolean>;
}) {
  const copy = useCopy().distillation;
  const editor = useDistillationEdit(change, proposal.revision, onEditing, review);
  const diff = { proposal, change, disabled, editor };
  const ids = [
    ...new Set([
      ...change.properties.flatMap((p) => p.quote_ids),
      ...change.lines.flatMap((l) => l.quote_ids),
    ]),
  ];
  return (
    <section className={styles.detail}>
      <div className={styles.noteHead}>
        <span>
          {change.kind === "create" ? copy.created : copy.updated} · {change.path}
        </span>
        {proposal.status === "proposed" && (
          <div>
            {editor.editing && (
              <Button disabled={disabled} onClick={() => editor.start(false)}>
                {copy.cancel}
              </Button>
            )}
            <Button
              disabled={disabled}
              onClick={() => (editor.editing ? void editor.save() : editor.start(true))}
            >
              {editor.editing ? copy.done : copy.edit}
            </Button>
          </div>
        )}
      </div>
      <h2>{change.title}</h2>
      {change.warnings.map((warning) => (
        <p className={styles.warning} key={warning.code}>
          {warning.code === "word_limit"
            ? copy.longNote(warning.words ?? 0, warning.max_words ?? 0)
            : copy.warning(warning.code)}
        </p>
      ))}
      {editor.error && <p role="alert">{copy.invalidProperty}</p>}
      <PropertyDiff {...diff} />
      <TextDiff {...diff} />
      <h3>{copy.said}</h3>
      <DistillationQuotes proposal={proposal} ids={ids} />
    </section>
  );
}
