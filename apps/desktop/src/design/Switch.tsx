import type { ReactNode } from "react";
import { es } from "../i18n/es";
import { useRequested, type CheckedChange } from "./requested";
import styles from "./Switch.module.css";

export interface SwitchProps {
  checked: boolean;
  disabled?: boolean;
  /** Its name; with `children` the visible label (at the right, also pressable) names it instead. */
  label: string;
  children?: ReactNode;
  onCheckedChange: CheckedChange;
  /**
   * It depends on a permission or on the system: after a press it does not
   * move, it says «Esperando a macOS…» until `checked` changes or the request is refused.
   */
  waitsForSystem?: boolean;
}

export function Switch(props: SwitchProps) {
  const { checked, children, disabled, label, onCheckedChange, waitsForSystem } = props;
  const { waiting, request } = useRequested(checked, onCheckedChange);
  const held = waitsForSystem === true && waiting;
  return (
    <span className={styles.row}>
      <button
        aria-busy={held || undefined}
        aria-checked={checked}
        aria-label={children === undefined ? label : undefined}
        className={styles.switch}
        disabled={disabled}
        onClick={() => {
          if (held) return;
          if (waitsForSystem === true) request(!checked);
          else void onCheckedChange(!checked);
        }}
        role="switch"
        type="button"
      >
        <span aria-hidden="true" className={styles.track}>
          <span className={styles.knob} />
        </span>
        {children}
      </button>
      <span aria-live="polite" className={styles.waiting}>
        {held ? es.controls.waitingForMacos : null}
      </span>
    </span>
  );
}
