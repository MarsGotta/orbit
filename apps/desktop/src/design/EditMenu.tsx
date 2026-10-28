import { useCopy } from "../i18n/CopyProvider";
import { useId, type KeyboardEvent, type ReactNode } from "react";

import { AnchoredCard } from "./AnchoredCard";
import { Button } from "./Button";
import type { Feedback } from "./feedback";
import cardStyles from "./AnchoredCard.module.css";
import styles from "./EditMenu.module.css";

// The fields where ↵ means «Guardar»; in a text of several lines it is a new line.
const ONE_LINE = new Set(["text", "search", "email", "url", "tel", "number", "password"]);

export interface EditMenuProps {
  /** The control that opened it: the pencil, or the word to correct. */
  anchor: HTMLElement | null;
  /** 400 for an assistant's pencil, 420 for a transcript's word. */
  width: 400 | 420;
  align?: "start" | "end";
  /** Above what it edits whenever it fits there: a word in the lower half of the transcript. */
  above?: boolean;
  title: string;
  subtitle?: ReactNode;
  /** Beside the title: the assistant's avatar in the pencil. */
  leading?: ReactNode;
  /** Whether what is written now is a valid change; the main button is off until it is. */
  canSave: boolean;
  saveLabel?: string;
  feedback?: Feedback;
  /** Why the change cannot be saved, above the buttons. */
  error?: string;
  /** At the start of the buttons' row: «Más ajustes». */
  extra?: ReactNode;
  onSave: () => void;
  /** Esc, «Cancelar» or a press outside: nothing is saved. */
  onClose: () => void;
  children: ReactNode;
}

/** The menu that edits something in place, anchored to what it edits. */
export function EditMenu(props: EditMenuProps) {
  const es = useCopy();
  const { anchor, width, align, above, title, subtitle, leading, canSave, feedback } = props;
  const { error, extra } = props;
  const { saveLabel = es.controls.save, onSave, onClose, children } = props;
  const titleId = useId();
  const saving = feedback === "pending";
  const saveOnEnter = (event: KeyboardEvent<HTMLElement>) => {
    const field = event.target;
    if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
    if (!(field instanceof HTMLInputElement) || !ONE_LINE.has(field.type)) return;
    event.preventDefault();
    if (canSave && !saving) onSave();
  };
  return (
    <AnchoredCard
      above={above}
      align={align}
      anchor={anchor}
      labelledBy={titleId}
      onClose={onClose}
      onKeyDown={saveOnEnter}
      width={width}
    >
      <header className={leading ? styles.headWithLeading : styles.head}>
        {leading}
        <div className={styles.headText}>
          <h2 className={cardStyles.title} id={titleId}>
            {title}
          </h2>
          {subtitle && <p className={styles.subtitle}>{subtitle}</p>}
        </div>
      </header>
      <div className={styles.fields}>{children}</div>
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      <footer className={`${cardStyles.footer} ${extra ? styles.footerWithExtra : ""}`}>
        {extra}
        <span className={styles.spacer} />
        <Button onClick={onClose} size={32} variant="ghost">
          {es.controls.cancel}
        </Button>
        <Button disabled={!canSave} feedback={feedback} onClick={onSave} size={32} variant="accent">
          {saveLabel}
        </Button>
      </footer>
    </AnchoredCard>
  );
}
