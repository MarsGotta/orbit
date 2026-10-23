import { useId, type KeyboardEvent } from "react";
import type { AssistantColor } from "../../design/AssistantAvatar";
import { moveWithArrows } from "../../design/roving";
import { assistantsCopy as copy } from "../../i18n/es/assistants";
import { DESCRIPTION_MAX, NAME_MAX, type profileProblem } from "./assistants";
import styles from "./Assistants.module.css";

const COLORS: readonly AssistantColor[] = [1, 2, 3, 4];

/** The four colours as a radio group, each with its name, not only its swatch. */
export function ColorChoice({
  value,
  onChange,
  labelledBy,
  label,
}: {
  value: AssistantColor;
  onChange: (color: AssistantColor) => void;
  labelledBy?: string;
  label?: string;
}) {
  return (
    <div
      aria-label={label}
      aria-labelledby={labelledBy}
      className={styles.colors}
      role="radiogroup"
    >
      {COLORS.map((color) => (
        <button
          aria-checked={color === value}
          aria-label={copy.colors[color - 1]}
          className={styles.swatch}
          data-color={color}
          key={color}
          onClick={() => onChange(color)}
          onKeyDown={(event: KeyboardEvent<HTMLElement>) =>
            moveWithArrows(event, COLORS, value, onChange)
          }
          role="radio"
          tabIndex={color === value ? 0 : -1}
          type="button"
        >
          <span aria-hidden="true" className={styles.swatchDot} />
        </button>
      ))}
    </div>
  );
}

/** «Nombre» with how it is called, or why it cannot be saved, under it. */
export function NameField({
  value,
  problem,
  onChange,
}: {
  value: string;
  problem: ReturnType<typeof profileProblem>;
  onChange: (name: string) => void;
}) {
  const id = useId();
  const hint =
    problem === "empty"
      ? copy.nameEmpty
      : problem === "taken"
        ? copy.nameTaken
        : problem === "too_long"
          ? copy.nameLong(NAME_MAX)
          : copy.nameHint(value.trim());
  return (
    <div className={styles.menuField}>
      <label className={styles.fieldLabel} htmlFor={id}>
        {copy.name}
      </label>
      <input
        aria-describedby={`${id}-hint`}
        aria-invalid={problem !== null}
        className={styles.input}
        data-invalid={problem !== null || undefined}
        id={id}
        maxLength={NAME_MAX}
        onChange={(event) => onChange(event.target.value)}
        type="text"
        value={value}
      />
      <span className={problem === null ? styles.fieldHint : styles.fieldError} id={`${id}-hint`}>
        {hint}
      </span>
    </div>
  );
}

/** «Descripción» on one line, with its counter against the hub's 90. */
export function DescriptionField({
  value,
  onChange,
}: {
  value: string;
  onChange: (description: string) => void;
}) {
  const id = useId();
  return (
    <div className={styles.menuField}>
      <label className={styles.fieldLabel} htmlFor={id}>
        {copy.description}
      </label>
      <textarea
        aria-describedby={`${id}-hint`}
        className={styles.textarea}
        id={id}
        maxLength={DESCRIPTION_MAX}
        onChange={(event) => onChange(event.target.value.replace(/\n/g, " "))}
        rows={2}
        value={value}
      />
      <span className={styles.counterRow} id={`${id}-hint`}>
        <span>{copy.descriptionHint}</span>
        <span className={styles.counter}>
          {copy.counter(Array.from(value).length, DESCRIPTION_MAX)}
        </span>
      </span>
    </div>
  );
}

/** «Color», named for a screen reader by the label above it. */
export function ColorField({
  value,
  onChange,
}: {
  value: AssistantColor;
  onChange: (color: AssistantColor) => void;
}) {
  const id = useId();
  return (
    <div className={styles.menuField}>
      <span className={styles.fieldLabel} id={id}>
        {copy.color}
      </span>
      <ColorChoice labelledBy={id} onChange={onChange} value={value} />
    </div>
  );
}
