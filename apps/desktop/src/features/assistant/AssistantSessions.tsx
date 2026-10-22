import { useId } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { DaemonPort } from "../../bridge/port";
import { ListColumn } from "../../app/shell-slots";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";
import { Loading } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
import { useNow } from "../../use-now";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { turnInFlight } from "./conversation";
import { agentFor, agentsOf } from "./agents";
import { openingLine, useFirstLines } from "./first-lines";
import { sessionWhen } from "./session-title";
import { useAssistantSessions, type SessionsState } from "./useAssistantSessions";
import styles from "./Sessions.module.css";

// The dates say «Hoy» and «Ayer»: read again each minute so they turn over.
const DATE_REPAINT_MS = 60_000;

interface Row {
  when: string;
  agent?: string;
  line: string;
}

function RowText({ row }: { row: Row }) {
  return (
    <>
      <span className={styles.top}>
        <span className={styles.when}>{row.when}</span>
        {row.agent !== undefined && <span className={styles.agent}>{row.agent}</span>}
      </span>
      {row.line === "" ? (
        <Skeleton height={10} widths={["70%"]} />
      ) : (
        <span className={styles.line}>{row.line}</span>
      )}
    </>
  );
}

function SkeletonRows() {
  return (
    <>
      {[0, 1, 2, 3].map((index) => (
        <li className={styles.skeleton} key={index}>
          <Skeleton height={12} widths={["46%", index % 2 === 0 ? "82%" : "64%"]} />
        </li>
      ))}
    </>
  );
}

function Footer({
  state,
  more,
  retry,
}: {
  state: SessionsState;
  more: () => void;
  retry: () => void;
}) {
  const es = useSpanish();
  return (
    <div aria-live="polite" className={styles.footer}>
      {state.loaded && state.loading && <Loading size={12}>{es.assistant.loadingSessions}</Loading>}
      {state.error !== null && (
        <>
          <p role="alert">{state.error}</p>
          <Button onClick={retry} size={28}>
            {es.assistant.retrySessions}
          </Button>
        </>
      )}
      {!state.loading && state.error === null && state.next !== null && (
        <Button onClick={more} size={28} variant="ghost">
          {es.assistant.loadMoreSessions}
        </Button>
      )}
    </div>
  );
}

/** The rows of the list: the open conversation's, and each recent one's. */
function useRows(snapshot: AppSnapshot, items: ChatSessionSummary[]) {
  const es = useSpanish();
  const now = useNow(DATE_REPAINT_MS);
  const lines = useFirstLines();
  const { chat } = snapshot;
  const openId = chat.session_id;
  const named = agentsOf(snapshot).length > 1;
  const agentName = (agentId?: string | null) =>
    named ? agentFor(snapshot, agentId)?.name : undefined;
  const rowOf = (session: ChatSessionSummary): Row => ({
    when: sessionWhen(session.created_at_ms, now),
    agent: agentName(session.agent_id),
    line: lines[session.id] ?? es.assistant.messageCount(session.message_count),
  });
  const listed = items.find((session) => session.id === openId);
  // A conversation begun a moment ago is not listed yet: its question names it.
  const asked = !listed || listed.message_count === 0 ? openingLine(chat.turn) : null;
  const begun = chat.turn[0]?.at_ms;
  let open: Row | null = null;
  if (openId !== null) {
    const listedRow = listed ? rowOf(listed) : null;
    open = {
      when: listedRow?.when ?? (begun !== undefined ? sessionWhen(begun, now) : es.assistant.now),
      agent: agentName(chat.agent_id),
      line: lines[openId] ?? asked ?? listedRow?.line ?? "",
    };
  }
  const recent = items
    .filter((session) => session.id !== openId)
    .map((session) => ({ session, row: rowOf(session) }));
  return { open, recent };
}

/** «Recientes»: each conversation a button that opens it with its agent. */
function Recent({
  recent,
  send,
}: {
  recent: { session: ChatSessionSummary; row: Row }[];
  send: (command: AppCommand) => unknown;
}) {
  const es = useSpanish();
  const recentId = useId();
  if (recent.length === 0) return null;
  return (
    <>
      <h3 className={styles.group} id={recentId}>
        {es.assistant.recentSessions}
      </h3>
      <ul aria-labelledby={recentId} className={styles.list}>
        {recent.map(({ session, row }) => (
          <li key={session.id}>
            <button
              className={styles.row}
              onClick={() =>
                send({
                  command: "session_open",
                  id: session.id,
                  ...(session.agent_id !== undefined ? { agent_id: session.agent_id } : {}),
                })
              }
              type="button"
            >
              <RowText row={row} />
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}

/**
 * The conversations, in the window's list column (`ListColumn`): «Nueva», the open one on top, then
 * «Recientes». Each says when it began, whose it is when the hub has several
 * agents, and its first sentence, never its date again.
 */
export function AssistantSessions({
  snapshot,
  send,
  read,
}: {
  snapshot: AppSnapshot;
  send: (command: AppCommand) => unknown;
  read?: DaemonPort["readSessionsPage"];
}) {
  const es = useSpanish();
  const { chat } = snapshot;
  const { state, more, retry } = useAssistantSessions(
    chat.session_id ?? undefined,
    turnInFlight(chat),
    read,
  );
  const { open, recent } = useRows(snapshot, state.items);
  return (
    <ListColumn
      actions={
        <Button
          aria-label={es.assistant.newSession}
          onClick={() => send({ command: "new_session" })}
          size={28}
        >
          <Icon name="plus" size={14} />
          {es.assistant.newShort}
        </Button>
      }
      label={es.assistant.conversations}
      title={es.assistant.conversations}
    >
      {open && (
        <div aria-current="page" className={styles.row} data-open>
          <RowText row={open} />
        </div>
      )}
      {!state.loaded && state.error === null ? (
        <ul aria-busy="true" aria-label={es.assistant.loadingSessions} className={styles.list}>
          <SkeletonRows />
        </ul>
      ) : (
        <Recent recent={recent} send={send} />
      )}
      <Footer more={more} retry={retry} state={state} />
    </ListColumn>
  );
}
