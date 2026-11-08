import { useCopy } from "../i18n/CopyProvider";
import { useId, type InputHTMLAttributes } from "react";

import styles from "./Field.module.css";

export interface FieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  label: string;
  help?: string;
  /** 36 standard; 40 in the first run and the composer. */
  size?: 36 | 40;
  /** A field that saves itself says «✓ Guardado» once it has, after leaving it. */
  saved?: boolean;
}

export function Field({ label, help, size = 36, saved, id, className, ...props }: FieldProps) {
  const es = useCopy();
  const fallbackId = useId();
  const inputId = id ?? fallbackId;
  const helpId = `${inputId}-help`;
  return (
    <div className={`${styles.field} ${className ?? ""}`}>
      <label className={styles.label} htmlFor={inputId}>
        {label}
      </label>
      <input
        aria-describedby={help ? helpId : undefined}
        className={`${styles.input} ${size === 40 ? styles.large : ""}`}
        id={inputId}
        {...props}
      />
      <div className={styles.below}>
        {help && (
          <span className={styles.help} id={helpId}>
            {help}
          </span>
        )}
        <span aria-live="polite" className={styles.saved}>
          {saved ? es.controls.saved : null}
        </span>
      </div>
    </div>
  );
}
