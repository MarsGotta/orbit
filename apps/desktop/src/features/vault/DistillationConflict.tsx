import { useCopy } from "../../i18n/CopyProvider";
import type { Distillation } from "../../bridge/generated/Distillation";
import type { DistillationConflict as Conflict } from "../../bridge/generated/DistillationConflict";
import type { ConflictChoice } from "../../bridge/generated/ConflictChoice";
import { noteName, valueText } from "./distillation-format";
import { DistillationQuotes } from "./DistillationQuotes";
import styles from "./Distillation.module.css";
export function DistillationConflict({
  proposal,
  conflict,
  disabled,
  choose,
}: {
  proposal: Distillation;
  conflict: Conflict;
  disabled: boolean;
  choose: (choice: ConflictChoice) => void;
}) {
  const copy = useCopy().distillation;
  const options = [
    ["incoming", copy.use(valueText(conflict.incoming)), copy.useHelp],
    ["current", copy.keep(valueText(conflict.current)), copy.keepHelp],
    ["both", copy.both, copy.bothHelp],
  ] as const;
  return (
    <section className={styles.detail}>
      <p className={styles.warning}>
        {copy.decide} · {copy.conflictHead}
      </p>
      <h2>{conflict.property}</h2>
      <p>{copy.conflictWhere(noteName(proposal, conflict.path), conflict.property)}</p>
      <div className={styles.comparison}>
        <div>
          <h3>{copy.current}</h3>
          <strong>{valueText(conflict.current)}</strong>
        </div>
        <div>
          <h3>{copy.incoming}</h3>
          <strong>{valueText(conflict.incoming)}</strong>
          <DistillationQuotes proposal={proposal} ids={conflict.quote_ids} />
        </div>
      </div>
      <fieldset className={styles.options} disabled={disabled || proposal.status !== "proposed"}>
        <legend className={styles.visuallyHidden}>{copy.decide}</legend>
        {options.map(([value, label, help]) => (
          <label key={value} data-selected={conflict.choice === value}>
            <input
              type="radio"
              name={`conflict-${conflict.id}`}
              checked={conflict.choice === value}
              onChange={() => choose(value)}
              aria-label={label}
            />
            <span>
              {label}
              <small>{help}</small>
            </span>
          </label>
        ))}
      </fieldset>
    </section>
  );
}
