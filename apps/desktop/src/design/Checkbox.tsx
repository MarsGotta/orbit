import type { ReactNode } from "react";
import { es } from "../i18n/es";
import { Icon } from "./Icon";
import { useRequested, type CheckedChange } from "./requested";
import styles from "./Checkbox.module.css";

export interface CheckboxProps {
  checked: boolean;
  disabled?: boolean;
  /** The visible label, which names it and presses it too. */
  children: ReactNode;
  onCheckedChange: CheckedChange;
}

/**
 * An action or a pending item: a press shows the new state at once, struck
 * through, with «Guardando…» beside it until the hub confirms. It never turns
 * back by itself; only a refusal puts back what the hub has.
 */
export function Checkbox({ checked, children, disabled, onCheckedChange }: CheckboxProps) {
  const { requested, waiting, request } = useRequested(checked, onCheckedChange);
  const shown = requested ?? checked;
  return (
    <span className={styles.row}>
      <button
        aria-busy={waiting || undefined}
        aria-checked={shown}
        className={styles.checkbox}
        disabled={disabled}
        onClick={() => request(!shown)}
        role="checkbox"
        type="button"
      >
        <span aria-hidden="true" className={styles.box}>
          {shown && <Icon name="check" size={14} />}
        </span>
        <span className={styles.label}>{children}</span>
      </button>
      <span aria-live="polite" className={styles.saving}>
        {waiting ? es.controls.saving : null}
      </span>
    </span>
  );
}
