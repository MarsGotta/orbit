import styles from "./Toggle.module.css";

export interface ToggleProps {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onCheckedChange: (checked: boolean) => void;
}

export function Toggle({ checked, disabled, label, onCheckedChange }: ToggleProps) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={`${styles.toggle} ${checked ? styles.checked : ""}`}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
      role="switch"
      type="button"
    >
      <span aria-hidden="true" className={styles.knob} />
    </button>
  );
}
