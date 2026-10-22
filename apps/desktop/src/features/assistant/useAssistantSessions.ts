import { useCallback, useEffect, useRef, useState } from "react";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { DaemonPort } from "../../bridge/port";
import type { AssistantCursor, SessionPage } from "../../bridge/reads";
import { es } from "../../i18n/es";

export type SessionsState = {
  items: ChatSessionSummary[];
  total: number | null;
  next: AssistantCursor | null;
  loading: boolean;
  /** The first page has arrived: until then the list shows its shape. */
  loaded: boolean;
  error: string | null;
};

function fromAnswer(answer: SessionPage, previous: SessionsState, first: boolean): SessionsState {
  if (answer.state === "absent")
    return {
      ...previous,
      total:
        answer.refusal.code === "hub_sessions_unpaged" && typeof answer.refusal.total === "number"
          ? answer.refusal.total
          : previous.total,
      loading: false,
      error: es.assistant.sessionsFailed,
    };
  // An answer that says nothing leaves the list unread, not empty: it is a
  // failed read, which «Reintentar» asks again.
  if (answer.state === "unknown")
    return { ...previous, loading: false, error: es.assistant.sessionsFailed };
  const seen = new Set(first ? [] : previous.items.map((session) => session.id));
  const items = first ? [] : [...previous.items];
  for (const session of answer.value.items) {
    if (!seen.has(session.id)) items.push(session);
    seen.add(session.id);
  }
  return {
    items,
    total: answer.value.total,
    next: answer.value.next,
    loading: false,
    loaded: true,
    error: null,
  };
}

/**
 * The conversations as the hub lists them, newest first. Read again when
 * another conversation opens and when a turn ends, so a new conversation and
 * its date appear in the list.
 */
export function useAssistantSessions(
  sessionId: string | undefined,
  streaming: boolean,
  read?: DaemonPort["readSessionsPage"],
) {
  const [state, setState] = useState<SessionsState>({
    items: [],
    total: null,
    next: null,
    loading: false,
    loaded: read === undefined,
    error: null,
  });
  const [retryToken, setRetryToken] = useState(0);
  // The first page is read whatever happens; then again once a turn has ended.
  const loaded = useRef(false);
  loaded.current = state.loaded;
  useEffect(() => {
    if (streaming && loaded.current) return;
    let current = true;
    setState((previous) => ({ ...previous, loading: Boolean(read), error: null }));
    if (read) {
      void read(null)
        .then((answer) => {
          if (current) setState((previous) => fromAnswer(answer, previous, true));
        })
        .catch(() => {
          if (current)
            setState((previous) => ({
              ...previous,
              loading: false,
              error: es.assistant.sessionsFailed,
            }));
        });
    }
    return () => {
      current = false;
    };
  }, [read, sessionId, streaming, retryToken]);

  const more = useCallback(() => {
    if (!read || !state.next || state.loading) return;
    const cursor = state.next;
    setState((previous) => ({ ...previous, loading: true, error: null }));
    void read(cursor)
      .then((answer) => setState((previous) => fromAnswer(answer, previous, false)))
      .catch(() =>
        setState((previous) => ({
          ...previous,
          loading: false,
          error: es.assistant.sessionsFailed,
        })),
      );
  }, [read, state.loading, state.next]);
  const retry = useCallback(() => setRetryToken((previous) => previous + 1), []);
  return { state, more, retry };
}
