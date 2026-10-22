import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { Icon } from "./Icon";
import styles from "./MultiSelect.module.css";

export interface MultiSelectOption {
  id: string;
  name: string;
  /** A line under its name: «Revisión semanal y Comité · 24 palabras». */
  detail?: string;
  /** A small label beside its name: «esta reunión». */
  tag?: string;
}

export interface MultiSelectProps {
  /** Shown above it, and the group's name: «Se añade a · puedes elegir varios». */
  label: string;
  options: MultiSelectOption[];
  selected: string[];
  onChange: (selected: string[]) => void;
  /** What it says with none chosen: «Elige uno o varios». */
  placeholder: string;
  /** The last row of the open list: «+ Crear un diccionario nuevo». */
  createLabel: string;
  onCreate: () => void;
}

/** Arrow keys between the rows of the open list; the first and last stay put. */
function moveFocus(event: KeyboardEvent<HTMLElement>) {
  const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
  if (step === undefined) return;
  event.preventDefault();
  const rows = Array.from(event.currentTarget.querySelectorAll<HTMLElement>("button"));
  const index = rows.indexOf(document.activeElement as HTMLElement);
  rows[Math.max(0, Math.min(rows.length - 1, index + step))]?.focus();
}

/**
 * Several choices in a small space. Closed, the chosen ones as chips; open, a
 * checkbox for each option, in the flow of what holds it, with a row to create
 * a new one at the end. Esc closes the list only, not the menu it sits in.
 */
export function MultiSelect(props: MultiSelectProps) {
  const { label, options, selected, placeholder } = props;
  const [open, setOpen] = useState(false);
  const labelId = useId();
  const listId = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const chosen = options.filter((option) => selected.includes(option.id));
  return (
    <div className={styles.select}>
      <span className={styles.label} id={labelId}>
        {label}
      </span>
      <button
        aria-controls={open ? listId : undefined}
        aria-expanded={open}
        aria-labelledby={`${labelId} ${listId}-chips`}
        className={styles.trigger}
        onClick={() => setOpen(!open)}
        ref={trigger}
        type="button"
      >
        <span className={styles.chips} id={`${listId}-chips`}>
          {chosen.length === 0 ? (
            <span className={styles.placeholder}>{placeholder}</span>
          ) : (
            chosen.map((option) => (
              <span className={styles.chip} key={option.id}>
                {option.name}
              </span>
            ))
          )}
        </span>
        <span className={styles.chevron} data-open={open}>
          <Icon name="chevron" size={14} />
        </span>
      </button>
      {open && (
        <OptionList
          {...props}
          id={listId}
          onClose={() => {
            setOpen(false);
            trigger.current?.focus();
          }}
        />
      )}
    </div>
  );
}

/** The open list: a checkbox for each option and the row that creates one. */
function OptionList(props: MultiSelectProps & { id: string; onClose: () => void }) {
  const { label, options, selected, onChange, createLabel, onCreate, id, onClose } = props;
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>("button")?.focus();
  }, []);
  const toggle = (option: string) =>
    onChange(
      selected.includes(option) ? selected.filter((one) => one !== option) : [...selected, option],
    );
  return (
    <div
      aria-label={label}
      className={styles.list}
      id={id}
      onKeyDown={(event) => {
        if (event.key !== "Escape") return moveFocus(event);
        event.stopPropagation();
        onClose();
      }}
      ref={list}
      role="group"
    >
      {options.map((option) => (
        <button
          aria-checked={selected.includes(option.id)}
          className={styles.option}
          key={option.id}
          onClick={() => toggle(option.id)}
          role="checkbox"
          type="button"
        >
          <span aria-hidden="true" className={styles.box}>
            {selected.includes(option.id) && <Icon name="check" size={14} />}
          </span>
          <span className={styles.optionText}>
            <span className={styles.name}>
              {option.name}
              {option.tag && <span className={styles.tag}>{option.tag}</span>}
            </span>
            {option.detail && <span className={styles.detail}>{option.detail}</span>}
          </span>
        </button>
      ))}
      <button className={styles.create} onClick={onCreate} type="button">
        {createLabel}
      </button>
    </div>
  );
}
