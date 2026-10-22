import { useEffect, useState, type FormEvent } from "react";
import { Field } from "../../design/Field";
import { es } from "../../i18n/es";
import styles from "./BriefingView.module.css";

// Kept in this Mac's webview, per calendar series; it never reaches the
// daemon, the hub or the note.
const contextPrefix = "orbit.briefing.context.local.";

function readReminder(seriesKey: string): string {
  try {
    return window.localStorage.getItem(`${contextPrefix}${seriesKey}`) ?? "";
  } catch {
    return "";
  }
}

function saveReminder(seriesKey: string, value: string): void {
  try {
    if (value === "") window.localStorage.removeItem(`${contextPrefix}${seriesKey}`);
    else window.localStorage.setItem(`${contextPrefix}${seriesKey}`, value);
  } catch {
    // The draft stays on screen when the webview cannot keep local data.
  }
}

/**
 * «Recordatorio personal»: something to remember for this meeting, only on
 * this Mac. It saves itself on leaving the field or pressing ↵, and says so.
 */
export function PersonalReminder({ seriesKey }: { seriesKey: string }) {
  const [draft, setDraft] = useState(() => readReminder(seriesKey));
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    setDraft(readReminder(seriesKey));
    setSaved(false);
  }, [seriesKey]);
  const save = () => {
    const value = draft.trim();
    if (value === readReminder(seriesKey)) return;
    saveReminder(seriesKey, value);
    setSaved(true);
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    save();
  };
  return (
    <form className={styles.reminder} onSubmit={submit}>
      <Field
        help={es.briefing.reminderHelp}
        label={es.briefing.reminder}
        onBlur={save}
        onChange={(event) => {
          setDraft(event.target.value);
          setSaved(false);
        }}
        placeholder={es.briefing.reminderPlaceholder}
        saved={saved}
        value={draft}
      />
    </form>
  );
}
