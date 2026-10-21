import type { AppView as AppSnapshot } from "../state/app-view";
import type { ReactNode } from "react";
import { es } from "../i18n/es";
import type { AppCommand } from "../bridge/generated";
import type { ChatSessionSummary } from "../bridge/generated/ChatSessionSummary";
import { sessionTitle } from "../features/assistant/session-title";
import type { RowTone } from "../features/notes/note-row";
import { notesData } from "./notes-context";
import type { ViewId } from "../state/ui";
import styles from "./ContextPanel.module.css";

interface ContextRow {
  active?: boolean;
  initial: string;
  meta: string;
  subtitle: string;
  /** How the subtitle reads: a note recording or failed shows it. */
  tone?: RowTone;
  title: string;
  turnActive?: boolean;
  onSelect?: () => void;
}

type ContextGroup = { label: string; rows: ContextRow[] };
export type Send = (command: AppCommand) => void;

const sessionDate = new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short" });
const formatSessionDate = (atMs: number) => (atMs > 0 ? sessionDate.format(atMs) : "");

export interface ContextPanelData {
  count: string;
  groups: ContextGroup[];
  title: string;
}

export function contextForView(
  view: ViewId,
  snapshot?: AppSnapshot,
  send?: Send,
  sessions: ChatSessionSummary[] = [],
): ContextPanelData | null {
  if (view === "notes") return notesData(snapshot, send);
  if (view === "assistant") return assistantData(snapshot, send, sessions);
  return null;
}

/** The conversations as the hub lists them, each named by its date. */
function assistantData(
  snapshot: AppSnapshot | undefined,
  send: Send | undefined,
  sessions: ChatSessionSummary[],
): ContextPanelData {
  const openId = snapshot?.chat.session_id ?? null;
  const open = sessions.find((session) => session.id === openId);
  const recent = sessions.filter((session) => session.id !== openId);
  const activeRows: ContextRow[] = [];
  if (openId) {
    // A conversation begun a moment ago is not listed yet.
    const title = open ? sessionTitle(open.created_at_ms) : es.assistant.emptySessionTitle;
    activeRows.push({
      active: true,
      initial: title.slice(0, 1) || "O",
      meta: es.context.now,
      subtitle: open ? formatSessionDate(open.last_at_ms) : "",
      title,
      turnActive: Boolean(snapshot?.chat.activity),
    });
  }
  activeRows.push({
    initial: "+",
    meta: "",
    subtitle: es.assistant.newSession,
    title: es.assistant.newSession,
    onSelect: () => send?.({ command: "new_session" }),
  });
  const groups: ContextGroup[] = [{ label: es.assistant.activeSessions, rows: activeRows }];
  if (recent.length > 0) {
    groups.push({
      label: es.assistant.recentSessions,
      rows: recent.map((session) => {
        const title = sessionTitle(session.created_at_ms);
        return {
          initial: title.slice(0, 1) || "O",
          meta: formatSessionDate(session.last_at_ms),
          subtitle: title,
          title,
          onSelect: () => send?.({ command: "session_open", id: session.id }),
        };
      }),
    });
  }
  return {
    title: es.assistant.sessions,
    count: String(Math.max(sessions.length, openId ? 1 : 0)),
    groups,
  };
}

export function ContextPanel({
  data,
  footer,
  narrow = false,
}: {
  data: ContextPanelData;
  footer?: ReactNode;
  /** The notes list is 244 px wide, so the note keeps its reading width. */
  narrow?: boolean;
}) {
  return (
    <aside
      aria-label={es.shell.contextLabel}
      className={styles.panel}
      data-narrow={narrow || undefined}
    >
      <header className={styles.header}>
        <strong>{data.title}</strong>
        <span>{data.count}</span>
      </header>
      <div className={styles.groups}>
        {data.groups.map((group) => (
          <section className={styles.group} key={group.label}>
            <h2>{group.label}</h2>
            {group.rows.map((item, index) => (
              <button
                className={styles.row}
                data-active={item.active || undefined}
                data-turn-active={item.turnActive || undefined}
                key={`${group.label}-${index}-${item.title}`}
                onClick={item.onSelect}
                type="button"
              >
                <span className={styles.initial}>{item.initial}</span>
                <span className={styles.copy}>
                  <strong>
                    {item.title} <span aria-hidden="true">›</span>
                  </strong>
                  <small data-tone={item.tone}>{item.subtitle}</small>
                </span>
                <span className={styles.meta}>{item.meta}</span>
              </button>
            ))}
          </section>
        ))}
      </div>
      {footer}
    </aside>
  );
}
