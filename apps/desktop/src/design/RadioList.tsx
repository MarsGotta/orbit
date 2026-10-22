import styles from "./RadioList.module.css";

export interface RadioListProps<T extends string> {
  label: string;
  /** The form name that keeps the radios together. */
  name: string;
  options: readonly { value: T; label: string; help?: string }[];
  value: T;
  onChange: (value: T) => void;
}

/** A list of options with an explanation each: «Qué audio grabar» in Nueva grabación. */
export function RadioList<T extends string>(props: RadioListProps<T>) {
  const { label, name, options, value, onChange } = props;
  return (
    <div aria-label={label} className={styles.list} role="radiogroup">
      {options.map((option) => (
        <label className={styles.row} key={option.value}>
          <input
            checked={option.value === value}
            className={styles.radio}
            name={name}
            onChange={() => onChange(option.value)}
            type="radio"
            value={option.value}
          />
          <span className={styles.text}>
            <span className={styles.title}>{option.label}</span>
            {option.help && <span className={styles.help}>{option.help}</span>}
          </span>
        </label>
      ))}
    </div>
  );
}
