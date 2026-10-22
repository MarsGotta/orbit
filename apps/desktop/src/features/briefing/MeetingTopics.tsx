import { useEffect, useState, type FormEvent } from "react";
import type { BriefingNextSnapshot } from "../../bridge/generated/BriefingNextSnapshot";
import { TextInput } from "../../design/TextInput";
import { es } from "../../i18n/es";
import styles from "./BriefingView.module.css";

const contextPrefix = "orbit.briefing.context.local.";

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, value),
  };
}

try {
  if (window.localStorage == null) {
    Object.defineProperty(window, "localStorage", { configurable: true, value: memoryStorage() });
  }
} catch {
  Object.defineProperty(window, "localStorage", { configurable: true, value: memoryStorage() });
}

function readContext(seriesKey: string): string {
  try {
    return window.localStorage.getItem(`${contextPrefix}${seriesKey}`) ?? "";
  } catch {
    return "";
  }
}

function saveContext(seriesKey: string, value: string): void {
  try {
    window.localStorage.setItem(`${contextPrefix}${seriesKey}`, value);
  } catch {
    // Keep the in-memory draft visible when the webview cannot persist local data.
  }
}

export function MeetingTopics({ meeting }: { meeting: BriefingNextSnapshot }) {
  const [draft, setDraft] = useState(() => readContext(meeting.series_key));
  const [saved, setSaved] = useState(() => readContext(meeting.series_key));
  useEffect(() => {
    const context = readContext(meeting.series_key);
    setDraft(context);
    setSaved(context);
  }, [meeting.series_key]);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const value = draft.trim();
    saveContext(meeting.series_key, value);
    setSaved(value);
  };
  return (
    <div className={styles.topics}>
      <h3>{es.briefing.agendaTopics}</h3>
      <ol>
        {(meeting.previous?.open_items ?? []).map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ol>
      {saved && (
        <p className={styles.savedContext}>
          <strong>{es.briefing.personalNote}</strong>
          {saved}
        </p>
      )}
      <form onSubmit={submit}>
        <TextInput
          aria-label={es.briefing.personalNote}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={es.briefing.personalNotePlaceholder}
          value={draft}
          variant="dotted"
        />
      </form>
      <p className={styles.savedContext}>{es.briefing.personalNoteHelp}</p>
    </div>
  );
}
