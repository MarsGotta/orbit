import { moveWithArrows } from "./roving";
import styles from "./Segmented.module.css";

export interface SegmentedProps<T extends string> {
  label: string;
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
}

/** Día / 3 días / Semana / Mes / Agenda, Oscuro / Claro, Ocupada / Libre. */
export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
}: SegmentedProps<T>) {
  const values = options.map((option) => option.value);
  return (
    <div aria-label={label} className={styles.segmented} role="radiogroup">
      {options.map((option) => (
        <button
          aria-checked={option.value === value}
          className={styles.option}
          key={option.value}
          onClick={() => onChange(option.value)}
          onKeyDown={(event) => moveWithArrows(event, values, value, onChange)}
          role="radio"
          tabIndex={option.value === value ? 0 : -1}
          type="button"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
