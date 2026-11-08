import { useCopy } from "../../i18n/CopyProvider";
import { useId } from "react";

import type { MenuRow } from "./menus";
import styles from "./MarkdownEditor.module.css";

function MenuOption({
  active,
  onHover,
  onPick,
  row,
}: {
  active: boolean;
  onHover: () => void;
  onPick: () => void;
  row: MenuRow;
}) {
  return (
    <div
      aria-selected={active}
      className={styles.menuRow}
      onMouseDown={(event) => {
        // The text keeps the focus and the cursor.
        event.preventDefault();
        onPick();
      }}
      onMouseEnter={onHover}
      role="option"
    >
      <span aria-hidden="true" className={styles.menuIcon}>
        {row.icon}
      </span>
      <span className={styles.menuLabel}>
        <span>{row.label}</span>
        {row.detail !== undefined && <span className={styles.menuDetail}>{row.detail}</span>}
      </span>
      {row.hint !== undefined && row.hint !== "" && (
        <span aria-hidden="true" className={styles.menuHint}>
          {row.hint}
        </span>
      )}
    </div>
  );
}

/**
 * «Insertar», «Enlazar una nota» or «Mencionar», under the line being written.
 * The keyboard stays in the text: ↑ ↓ choose, ↵ inserts, Esc closes.
 */
export function EditorMenu({
  active,
  empty,
  onHover,
  onPick,
  place,
  rows,
  title,
}: {
  active: number;
  empty: string;
  onHover: (index: number) => void;
  onPick: (row: MenuRow) => void;
  /** Under the line, from its start, as the prototype hangs it. */
  place: { bottom: number };
  rows: MenuRow[];
  title: string;
}) {
  const es = useCopy();
  const id = useId();
  return (
    <div className={styles.menu} style={{ top: place.bottom + 6 }}>
      <div className={styles.menuTitle} id={id}>
        {title}
      </div>
      {rows.length === 0 ? (
        <p className={styles.menuEmpty}>{empty}</p>
      ) : (
        <div aria-labelledby={id} className={styles.menuRows} role="listbox">
          {rows.map((row, index) => (
            <MenuOption
              active={index === active}
              key={row.id}
              onHover={() => onHover(index)}
              onPick={() => onPick(row)}
              row={row}
            />
          ))}
        </div>
      )}
      <div aria-hidden="true" className={styles.menuKeys}>
        {es.editor.menuKeys.map((keys) => (
          <span key={keys}>{keys}</span>
        ))}
      </div>
    </div>
  );
}
