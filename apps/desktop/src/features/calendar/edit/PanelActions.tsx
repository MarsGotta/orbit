import { useState } from "react";
import type { CalendarSpan } from "../../../bridge/generated/CalendarSpan";
import { Button } from "../../../design/Button";
import { Popover } from "../../../design/Popover";
import { Spinner } from "../../../design/Spinner";
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
 * The panel's foot: «Borrar» on the left; on the right «Sin cambios» until
 * something changes, then the buttons that save it. In a series the scope is
 * the button the user presses: «Solo este» or «Este y los siguientes» (the
 * design's main one); ⌘↵ presses the first. A save says «Guardando…» until the calendar
 * has it, then «✓ Guardado en tu calendario». Deleting is undone, not confirmed.
 */
export function PanelActions({ editing, onDelete, onSave }: ActionsProps) {
  return (
    <footer className={styles.footer}>
      {editing.event !== null && <DeleteButton editing={editing} onDelete={onDelete} />}
      <div className={styles.footerSpacer} />
      <div aria-live="polite" className={styles.saveArea}>
        <SaveArea editing={editing} onSave={onSave} />
      </div>
    </footer>
  );
}

function SaveArea({ editing, onSave }: Omit<ActionsProps, "onDelete">) {
  const draft = editing.event === null;
  const dirty = isDirty(editing);
  if (editing.saving?.state === "pending")
    return (
      <span className={styles.saveState}>
        <Spinner size={12} />
        {es.calendar.write.saving}
      </span>
    );
  if (draft)
    return (
      <Button
        aria-keyshortcuts="Meta+Enter"
        disabledReason={dirty ? undefined : es.calendar.edit.needsTitle}
        onClick={() => onSave("this")}
        size={32}
        variant="accent"
      >
        {es.calendar.edit.create}
      </Button>
    );
  if (!dirty)
    return editing.saving?.state === "saved" ? (
      <span className={styles.saved}>{es.calendar.edit.saved}</span>
    ) : (
      <span className={styles.saveState}>{es.calendar.edit.noChanges}</span>
    );
  const choices = saveChoices(editing);
  const main = choices.length - 1;
  return (
    <>
      {choices.map((span, index) => (
        <Button
          // ⌘↵ presses the narrower scope, as before the redesign.
          aria-keyshortcuts={index === 0 ? "Meta+Enter" : undefined}
          key={span}
          onClick={() => onSave(span)}
          size={32}
          variant={index === main ? "accent" : "outline"}
        >
          {saveLabel(editing, span, choices.length)}
        </Button>
      ))}
    </>
  );
}

/** «Borrar»; in a series it asks which ones inside its own menu. */
function DeleteButton({ editing, onDelete }: Omit<ActionsProps, "onSave">) {
  const [open, setOpen] = useState(false);
  if (editing.event === null || !isSeries(editing.event))
    return (
      <Button className={styles.delete} onClick={() => onDelete("this")} size={32} variant="ghost">
        {es.calendar.edit.delete}
      </Button>
    );
  return (
    <Popover
      label={es.calendar.edit.delete}
      onOpenChange={setOpen}
      open={open}
      trigger={es.calendar.edit.delete}
      triggerClassName={styles.deleteTrigger}
    >
      <div className={styles.deleteMenu}>
        {(["this", "future"] as const).map((span) => (
          <button
            className={styles.deleteChoice}
            key={span}
            onClick={() => {
              setOpen(false);
              onDelete(span);
            }}
            type="button"
          >
            {span === "this" ? es.calendar.edit.deleteThis : es.calendar.edit.deleteFuture}
          </button>
        ))}
      </div>
    </Popover>
  );
}
