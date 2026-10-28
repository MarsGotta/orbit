import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useRef, useState, type FormEvent } from "react";
import type { BriefingReminder } from "../../bridge/generated/BriefingReminder";
import { Field } from "../../design/Field";

import type { Send } from "./send";
import styles from "./BriefingView.module.css";

// This Mac's copy, in its webview, by the calendar item, which a rename in
// Calendar keeps; an event without one, by its series (its title's words), as
// every reminder was kept before (`legacyPrefix`). The hub keeps it too, for
// every Mac of its owner (`reminder_save`); never the note. Each copy says
// when it was written (`editedKeyOf`): the latest edit wins.
const itemPrefix = "orbit.briefing.reminder.item.";
const legacyPrefix = "orbit.briefing.context.local.";
const editedItemPrefix = "orbit.briefing.reminder.edited.item.";
const editedSeriesPrefix = "orbit.briefing.reminder.edited.series.";

/**
 * The longest reminder, in characters: the hub's own bound
 * (`MAX_REMINDER_CHARS` in `server/orbit_hub/storage/reminders.py`,
 * contracts/constants.json), so the field never holds one the hub refuses.
 */
export const MAX_REMINDER_CHARS = 1_000;

interface Place {
  calendarItemId: string | null;
  seriesKey: string;
}

/** Why the webview did not keep it: `quota` when it has no room left. */
type Kept = { ok: true } | { ok: false; cause: "quota" | "unavailable" };

/** This Mac's copy and when it was written (0: before the time was kept). */
interface Copy {
  text: string;
  edited: number;
}

function keyOf(place: Place): string {
  return place.calendarItemId !== null
    ? `${itemPrefix}${place.calendarItemId}`
    : `${legacyPrefix}${place.seriesKey}`;
}

function editedKeyOf(place: Place): string {
  return place.calendarItemId !== null
    ? `${editedItemPrefix}${place.calendarItemId}`
    : `${editedSeriesPrefix}${place.seriesKey}`;
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

function readCopy(place: Place): Copy {
  let edited = 0;
  try {
    edited = Number(window.localStorage.getItem(editedKeyOf(place)) ?? 0) || 0;
  } catch {
    edited = 0;
  }
  return { text: readReminder(place), edited };
}

function saveReminder(place: Place, value: string, edited: number): Kept {
  try {
    const storage = window.localStorage;
    if (value === "") storage.removeItem(keyOf(place));
    else storage.setItem(keyOf(place), value);
    storage.setItem(editedKeyOf(place), String(edited));
    // Moved to the calendar item: the series copy would come back after a delete.
    if (place.calendarItemId !== null) storage.removeItem(`${legacyPrefix}${place.seriesKey}`);
    return { ok: true };
  } catch (error: unknown) {
    const quota = error instanceof DOMException && error.name === "QuotaExceededError";
    return { ok: false, cause: quota ? "quota" : "unavailable" };
  }
}

/** What the hub's copy calls for: take it (newer), send this Mac's (older or none), or nothing. */
function reconcile(hub: BriefingReminder, copy: Copy): "take" | "send" | null {
  if (hub.state === "unknown") return null;
  if (hub.state === "none") return copy.text !== "" || copy.edited > 0 ? "send" : null;
  if (hub.edited_at_ms > copy.edited) return "take";
  if (hub.edited_at_ms < copy.edited) return "send";
  // The same time: the hub kept one of two copies written before times were kept.
  return hub.text === copy.text ? null : "take";
}

interface Props extends Place {
  /** The hub's copy (`briefing.next.reminder`); absent until it was asked for. */
  hub?: BriefingReminder;
  /** The hub can be reached: when it comes back, it is asked again. */
  online: boolean;
  send: Send;
}

/**
 * The hub's copy against this Mac's: the hub is asked when the reminder is
 * shown and when the hub comes back; a newer copy from another Mac replaces
 * this one (`take`), and this Mac's newer one goes up, once each time the hub
 * is asked. Returns how to send this Mac's copy.
 */
function useHubCopy(
  place: Place,
  { hub, online, send }: Omit<Props, keyof Place>,
  editing: { current: boolean },
  take: (text: string) => void,
): (copy: Copy) => void {
  const { calendarItemId, seriesKey } = place;
  // The copy already sent since the hub was last asked, by when it was written.
  const sent = useRef<number | null>(null);
  const push = (copy: Copy) => {
    if (sent.current === copy.edited) return;
    sent.current = copy.edited;
    const command = {
      command: "reminder_save",
      series_key: seriesKey,
      text: copy.text,
      edited_at_ms: copy.edited,
    } as const;
    Promise.resolve(send(command)).catch(() => {
      // Kept here; sent again the next time the hub is asked.
      sent.current = null;
    });
  };
  useEffect(() => {
    sent.current = null;
    void send({ command: "reminder_read", series_key: seriesKey });
  }, [seriesKey, online]);
  useEffect(() => {
    if (hub === undefined || editing.current) return;
    const copy = readCopy(place);
    const action = reconcile(hub, copy);
    if (action === "send") push(copy);
    if (action === "take" && hub.state === "known") {
      saveReminder(place, hub.text, hub.edited_at_ms);
      take(hub.text);
    }
  }, [hub, calendarItemId, seriesKey]);
  return push;
}

/**
 * «Recordatorio personal»: something to remember for this meeting, only for
 * its owner, on every Mac. Every change is kept on this Mac at once, so
 * leaving the briefing, quitting or a fall do not take it; leaving the field
 * or pressing ↵ says «✓ Guardado» and sends it to the hub. A Mac that could
 * not send it sends it when the hub can be asked again; a newer copy from
 * another Mac replaces this one. What this Mac could not keep is said with
 * its cause, and stays on screen.
 */
export function PersonalReminder({ calendarItemId, seriesKey, ...hub }: Props) {
  const es = useCopy();
  const place = { calendarItemId, seriesKey };
  const [draft, setDraft] = useState(() => readReminder(place));
  const [saved, setSaved] = useState(false);
  const [kept, setKept] = useState<Kept>({ ok: true });
  const changed = useRef(false);
  const push = useHubCopy(place, hub, changed, setDraft);
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
    push(readCopy(place));
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
        maxLength={MAX_REMINDER_CHARS}
        onBlur={confirm}
        onChange={(event) => {
          const value = event.target.value;
          setDraft(value);
          setSaved(false);
          setKept(saveReminder(place, value.trim(), Date.now()));
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
