import { dateTime } from "../../i18n/locale";
import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useId } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Skeleton } from "../../design/Skeleton";

import { useNow } from "../../use-now";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { assistantHash } from "./assistant-route";
import { colorOf } from "./assistants";
import { openSession, sessionAgent } from "./AssistantHome";
import { agentsOf } from "./agents";
import { openingLine, useFirstLines } from "./first-lines";
import { sessionWhen } from "./session-title";
import styles from "./Sessions.module.css";

// The dates say «Hoy» and «Ayer»: read again each minute so they turn over.
const DATE_REPAINT_MS = 60_000;

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

/** The day a conversation is grouped under: «Hoy», «Ayer», «18 oct» (with its year, another year's). */
export function dayOf(atMs: number, nowMs: number): string {
  const at = new Date(atMs);
  const now = new Date(nowMs);
  if (sameDay(at, now)) return "Hoy";
  if (sameDay(at, new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1))) return "Ayer";
  return at.getFullYear() === now.getFullYear()
    ? getCopy().extras.shortDay(atMs)
    : `${getCopy().extras.shortDay(atMs)} ${at.getFullYear()}`;
}

const clock = dateTime({
  hour: "2-digit",
  minute: "2-digit",
});

export interface Row {
  id: string;
  /** Its first sentence, or how many messages it holds before that is known. */
  line: string;
  at: number;
  agent?: Agent;
  open: boolean;
  session?: ChatSessionSummary;
  waiting?: string;
}

/** The rows of the list: each listed one, and the open one before the hub lists it. */
export function useRows(snapshot: AppSnapshot, items: ChatSessionSummary[]): Row[] {
  const es = useCopy();
  const lines = useFirstLines();
  const { chat } = snapshot;
  const agents = agentsOf(snapshot);
  const primaryId = snapshot.settings.default_agent_id;
  const agentOf = (id?: string) => agents.find((agent) => agent.id === id);
  const openId = chat.session_id;
  const rows: Row[] = items.map((session) => ({
    id: session.id,
    line: lines[session.id] ?? es.assistant.messageCount(session.message_count),
    at: session.created_at_ms,
    agent: agentOf(sessionAgent(session, primaryId)),
    open: session.id === openId,
    session,
    waiting: chat.waiting_sessions?.[session.id],
  }));
  const listed = items.find((session) => session.id === openId);
  // A conversation begun a moment ago is not listed yet: its question names it.
  if (openId !== null && listed === undefined) {
    const asked = openingLine(chat.turn);
    rows.unshift({
      id: openId,
      line: lines[openId] ?? asked ?? "",
      at: chat.turn[0]?.at_ms ?? Date.now(),
      agent: agentOf(chat.agent_id ?? primaryId),
      open: true,
    });
  } else if (listed !== undefined && listed.message_count === 0) {
    const asked = openingLine(chat.turn);
    const open = rows.find((row) => row.open);
    if (open !== undefined && asked !== null && lines[openId ?? ""] === undefined)
      open.line = asked;
  }
  return rows;
}

/** One conversation: its assistant's avatar, its first sentence, its time and whose it is. */
function ConversationRow({
  row,
  named,
  agents,
  send,
  now,
}: {
  row: Row;
  named: boolean;
  agents: readonly Agent[];
  send: (command: AppCommand) => unknown;
  now: number;
}) {
  const assistantsCopy = useCopy().extras.assistantsCopy;
  const es = useCopy();
  const when = sessionWhen(row.at, now);
  const name = named ? row.agent?.name : undefined;
  return (
    <a
      aria-current={row.open ? "page" : undefined}
      aria-label={assistantsCopy.rowLabel(row.line === "" ? when : row.line, when, name)}
      className={styles.row}
      data-open={row.open || undefined}
      href={assistantHash({ screen: "conversation", sessionId: row.id })}
      onClick={() => {
        if (row.session !== undefined && !row.open) openSession(row.session, send);
      }}
    >
      {row.agent !== undefined && named && (
        <span className={styles.rowAvatar}>
          <AssistantAvatar color={colorOf(row.agent, agents)} name={row.agent.name} size={20} />
        </span>
      )}
      <span className={styles.rowText}>
        <span className={styles.top}>
          {row.line === "" ? (
            <Skeleton height={10} widths={["70%"]} />
          ) : (
            <span className={styles.line}>{row.line}</span>
          )}
          <span className={styles.time}>{clock.format(new Date(row.at))}</span>
        </span>
        {name !== undefined && <span className={styles.agent}>{name}</span>}
        {row.waiting !== undefined && (
          <span className={styles.agent}>
            {row.waiting === "approval" ? es.assistant.waitsPermission : es.assistant.asksSomething}
          </span>
        )}
      </span>
    </a>
  );
}

/** «Conversaciones», grouped by the day each began. */
export function Conversations({
  rows,
  snapshot,
  send,
}: {
  rows: Row[];
  snapshot: AppSnapshot;
  send: (command: AppCommand) => unknown;
}) {
  const now = useNow(DATE_REPAINT_MS);
  const baseId = useId();
  const agents = agentsOf(snapshot);
  const named = agents.length > 1;
  const days: { day: string; rows: Row[] }[] = [];
  for (const row of rows) {
    const day = dayOf(row.at, now);
    const last = days.at(-1);
    if (last?.day === day) last.rows.push(row);
    else days.push({ day, rows: [row] });
  }
  return (
    <>
      {days.map(({ day, rows: dayRows }, index) => (
        <div key={day}>
          <h4 className={styles.day} id={`${baseId}-${index}`}>
            {day}
          </h4>
          <ul aria-labelledby={`${baseId}-${index}`} className={styles.list}>
            {dayRows.map((row) => (
              <li key={row.id}>
                <ConversationRow agents={agents} named={named} now={now} row={row} send={send} />
              </li>
            ))}
          </ul>
        </div>
      ))}
    </>
  );
}
