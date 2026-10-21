import type { CalendarSpan } from "../../../bridge/generated/CalendarSpan";
import { Button } from "../../../design/Button";
import { Kbd } from "../../../design/Kbd";
import { es } from "../../../i18n/es";
import { isSeries } from "./commands";
import { type Editing, isDirty, saveChoices } from "./useEventEditor";
import styles from "./EventPanel.module.css";

interface ActionsProps {
  editing: Editing;
  onDelete: (span: CalendarSpan) => void;
  onSave: (span: CalendarSpan) => void;
}

function saveLabel(editing: Editing, span: CalendarSpan, choices: number): string {
  if (editing.event === null) return es.calendar.edit.create;
  if (choices === 1 && !isSeries(editing.event)) return es.calendar.edit.save;
  return span === "this" ? es.calendar.edit.saveThis : es.calendar.edit.saveFuture;
}

/**
 * The buttons say what they will do (P9-22). In a series the scope is the
 * button Mars presses, not a question afterwards; the first is the one ⌘↵
 * presses. Delete needs no confirmation: it is undone from its notice.
 */
export function PanelActions({ editing, onDelete, onSave }: ActionsProps) {
  const choices = saveChoices(editing);
  const draft = editing.event === null;
  const dirty = isDirty(editing);
  const series = editing.event !== null && isSeries(editing.event);
  return (
    <div className={styles.actions}>
      {(dirty || draft) &&
        choices.map((span, index) => (
          <Button
            disabled={!dirty}
            key={span}
            onClick={() => onSave(span)}
            size={30}
            title={dirty ? undefined : es.calendar.edit.needsTitle}
            variant={index === 0 ? "accent" : "outline"}
          >
            {saveLabel(editing, span, choices.length)}
            {index === 0 && <Kbd>{es.calendar.edit.saveShortcut}</Kbd>}
          </Button>
        ))}
      {draft && !dirty && <span className={styles.hint}>{es.calendar.edit.needsTitle}</span>}
      {!draft &&
        (series ? (["this", "future"] as const) : (["this"] as const)).map((span) => (
          <Button
            className={styles.delete}
            key={`delete-${span}`}
            onClick={() => onDelete(span)}
            size={30}
            variant="ghost"
          >
            {series
              ? span === "this"
                ? es.calendar.edit.deleteThis
                : es.calendar.edit.deleteFuture
              : es.calendar.edit.delete}
          </Button>
        ))}
    </div>
  );
}
