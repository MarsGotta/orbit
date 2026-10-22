import { useEffect, useRef, useState, type FormEvent } from "react";
import { Field } from "../../design/Field";
import { es } from "../../i18n/es";
import styles from "./BriefingView.module.css";

// Kept in this Mac's webview only; it never reaches the daemon, the hub or
// the note. By the calendar item, which a rename in Calendar keeps; an event
// without one, by its series (its title's words), as every reminder was kept
// before (`legacyPrefix`).
const itemPrefix = "orbit.briefing.reminder.item.";
const legacyPrefix = "orbit.briefing.context.local.";

interface Place {
  calendarItemId: string | null;
  seriesKey: string;
}

/** Why the webview did not keep it: `quota` when it has no room left. */
type Kept = { ok: true } | { ok: false; cause: "quota" | "unavailable" };

function keyOf(place: Place): string {
  return place.calendarItemId !== null
    ? `${itemPrefix}${place.calendarItemId}`
    : `${legacyPrefix}${place.seriesKey}`;
}

function readReminder(place: Place): string {
  try {
    const storage = window.localStorage;
    const own = storage.getItem(keyOf(place));
    if (own !== null || place.calendarItemId === null) return own ?? "";
    return storage.getItem(`${legacyPrefix}${place.seriesKey}`) ?? "";
  } catch {
    return "";
  }
}

function saveReminder(place: Place, value: string): Kept {
  try {
    const storage = window.localStorage;
    if (value === "") storage.removeItem(keyOf(place));
    else storage.setItem(keyOf(place), value);
    // Moved to the calendar item: the series copy would come back after a delete.
    if (place.calendarItemId !== null) storage.removeItem(`${legacyPrefix}${place.seriesKey}`);
    return { ok: true };
  } catch (error: unknown) {
    const quota = error instanceof DOMException && error.name === "QuotaExceededError";
    return { ok: false, cause: quota ? "quota" : "unavailable" };
  }
}

/**
 * «Recordatorio personal»: something to remember for this meeting, only on
 * this Mac. Every change is kept at once, so leaving the briefing, quitting
 * or a fall do not take it; leaving the field or pressing ↵ says «✓ Guardado».
 * What this Mac could not keep is said with its cause, and stays on screen.
 */
export function PersonalReminder({ calendarItemId, seriesKey }: Place) {
  const place = { calendarItemId, seriesKey };
  const [draft, setDraft] = useState(() => readReminder(place));
  const [saved, setSaved] = useState(false);
  const [kept, setKept] = useState<Kept>({ ok: true });
  const changed = useRef(false);
  useEffect(() => {
    setDraft(readReminder(place));
    setSaved(false);
    setKept({ ok: true });
    changed.current = false;
  }, [calendarItemId, seriesKey]);
  const confirm = () => {
    if (!changed.current) return;
    changed.current = false;
    setSaved(kept.ok);
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    confirm();
  };
  return (
    <form className={styles.reminder} onSubmit={submit}>
      <Field
        help={es.briefing.reminderHelp}
        label={es.briefing.reminder}
        onBlur={confirm}
        onChange={(event) => {
          const value = event.target.value;
          setDraft(value);
          setSaved(false);
          setKept(saveReminder(place, value.trim()));
          changed.current = true;
        }}
        placeholder={es.briefing.reminderPlaceholder}
        saved={saved}
        value={draft}
      />
      {!kept.ok && (
        <p className={styles.reminderNotKept} role="alert">
          {es.briefing.reminderNotKept(kept.cause)}
        </p>
      )}
    </form>
  );
}
