import { useCopy } from "orbit-ui/i18n/CopyProvider";
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import type { DistillationChange } from "orbit-ui/bridge/generated/DistillationChange";
import type { DistillationEditor } from "./use-distillation-edit";
import { valueText } from "./distillation-format";
import { DistillationQuotes } from "./DistillationQuotes";
import { DiffMarkdown } from "./DiffMarkdown";
import styles from "./Distillation.module.css";
type Props = {
  proposal: Distillation;
  change: DistillationChange;
  editor: DistillationEditor;
  disabled: boolean;
};
export function PropertyDiff({ change, editor, disabled }: Props) {
  const copy = useCopy().distillation;
  return (
    <>
      <h3>{copy.props}</h3>
      <dl className={styles.properties}>
        {change.properties.map((prop) => (
          <div key={prop.key}>
            <dt>{prop.key}</dt>
            <dd>
              <del>{valueText(prop.before)}</del> →{" "}
              {editor.editing ? (
                <input
                  aria-label={prop.key}
                  disabled={disabled}
                  value={editor.draft.properties[prop.key] ?? ""}
                  onChange={(e) => editor.update("properties", prop.key, e.target.value)}
                />
              ) : (
                <ins>{valueText(prop.after)}</ins>
              )}
            </dd>
          </div>
        ))}
      </dl>
    </>
  );
}
export function TextDiff({ change, proposal, editor, disabled }: Props) {
  const copy = useCopy().distillation;
  return (
    <>
      <h3>{copy.text}</h3>
      {change.lines.map((line, i) => (
        <section className={styles.diff} key={i}>
          <h4>{line.section}</h4>
          {line.before !== "" && (
            <del>
              <DiffMarkdown text={line.before} />
            </del>
          )}
          <div className={styles.addition}>
            {editor.editing ? (
              <textarea
                aria-label={line.section}
                disabled={disabled}
                value={editor.draft.lines[String(i)] ?? ""}
                onChange={(e) => editor.update("lines", String(i), e.target.value)}
              />
            ) : (
              <ins>
                <DiffMarkdown text={line.after} />
              </ins>
            )}
            <DistillationQuotes compact proposal={proposal} ids={line.quote_ids} />
          </div>
        </section>
      ))}
    </>
  );
}
