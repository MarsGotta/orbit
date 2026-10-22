import { useId, type ReactNode } from "react";
import { moveWithArrows } from "./roving";
import styles from "./Tabs.module.css";

export interface TabsProps<T extends string> {
  label: string;
  /** The count goes in the label: «Acciones · 3». */
  tabs: readonly { id: T; label: string }[];
  selected: T;
  onSelect: (id: T) => void;
  /** The chosen tab's content. */
  children: ReactNode;
}

export function Tabs<T extends string>({
  label,
  tabs,
  selected,
  onSelect,
  children,
}: TabsProps<T>) {
  const prefix = useId();
  const ids = tabs.map((tab) => tab.id);
  return (
    <>
      <div aria-label={label} className={styles.list} role="tablist">
        {tabs.map((tab) => (
          <button
            aria-controls={`${prefix}-panel`}
            aria-selected={tab.id === selected}
            className={styles.tab}
            id={`${prefix}-${tab.id}`}
            key={tab.id}
            onClick={() => onSelect(tab.id)}
            onKeyDown={(event) => moveWithArrows(event, ids, selected, onSelect)}
            role="tab"
            tabIndex={tab.id === selected ? 0 : -1}
            type="button"
          >
            {tab.label}
          </button>
        ))}
      </div>
      <div aria-labelledby={`${prefix}-${selected}`} id={`${prefix}-panel`} role="tabpanel">
        {children}
      </div>
    </>
  );
}
