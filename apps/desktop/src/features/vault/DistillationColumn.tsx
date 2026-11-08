import { sourceHeading } from "./distillation-presentation";
import { useCopy } from "orbit-ui/i18n/CopyProvider";
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import type { DistillationReview } from "orbit-ui/bridge/generated/DistillationReview";
import { countChanges, countNotes, valueText } from "./distillation-format";
import styles from "./Distillation.module.css";
export function DistillationColumn({
  proposal,
  selected,
  select,
  review,
  disabled,
}: {
  proposal: Distillation;
  selected: string;
  select: (id: string) => void;
  review: (review: DistillationReview) => void;
  disabled: boolean;
}) {
  const copy = useCopy().distillation;
  return (
    <aside className={styles.column}>
      <h2>{copy.decide}</h2>
      <ConflictLinks proposal={proposal} selected={selected} select={select} disabled={disabled} />
      <h2>{copy.noteChanges(countNotes(proposal))}</h2>
      {proposal.changes.map((change) => (
        <div className={styles.changeRow} key={change.id}>
          <input
            aria-label={copy.include(change.title)}
            checked={change.include}
            disabled={disabled || proposal.status !== "proposed"}
            onChange={(e) =>
              review({
                revision: proposal.revision,
                changes: { [change.id]: { include: e.target.checked } },
              })
            }
            type="checkbox"
          />
          <button
            className={styles.select}
            aria-label={`${change.title} · ${copy.changes(countChanges([{ ...change, include: true }]))}`}
            aria-pressed={selected === change.id}
            disabled={disabled}
            onClick={() => select(change.id)}
          >
            <span>
              {change.title} · {copy.changes(countChanges([{ ...change, include: true }]))}
            </span>
            <small>{change.lines.map((line) => line.section).join(" · ")}</small>
          </button>
        </div>
      ))}
      {proposal.new_source !== null && (
        <section>
          <h2>{sourceHeading(proposal, copy)}</h2>
          <p>
            {copy.sourceItem}
            <small>{copy.sourceSub}</small>
          </p>
        </section>
      )}
      <Footnotes proposal={proposal} />
    </aside>
  );
}

/** Notes the hub left out because they did not validate, then what Orbit also writes. */
function Footnotes({ proposal }: { proposal: Distillation }) {
  const copy = useCopy().distillation;
  return (
    <>
      {proposal.dropped.map((drop, index) => (
        <p className={styles.warning} key={`${index}:${drop.path}`}>
          {copy.dropped(drop.path, drop.code)}
        </p>
      ))}
      <p className={styles.index}>{copy.alsoIndex}</p>
    </>
  );
}

function ConflictLinks({
  proposal,
  selected,
  select,
  disabled,
}: {
  proposal: Distillation;
  selected: string;
  select: (id: string) => void;
  disabled: boolean;
}) {
  const copy = useCopy().distillation;
  return (
    <>
      {" "}
      {proposal.conflicts.map((conflict) => (
        <button
          className={styles.select}
          aria-pressed={selected === conflict.id}
          disabled={disabled}
          key={conflict.id}
          onClick={() => select(conflict.id)}
        >
          <span>{conflict.property}</span>
          <small data-pending={conflict.choice === null}>
            {conflict.choice === null
              ? copy.missing
              : conflict.choice === "both"
                ? copy.both
                : conflict.choice === "incoming"
                  ? copy.use(valueText(conflict.incoming))
                  : copy.keep(valueText(conflict.current))}
          </small>
        </button>
      ))}
    </>
  );
}
