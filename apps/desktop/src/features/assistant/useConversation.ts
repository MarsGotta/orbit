import type { AppView as AppSnapshot } from "../../state/app-view";
import { useCallback, useEffect, useRef, useState } from "react";

import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { DaemonPort } from "../../bridge/port";
import type { AssistantCursor, MessagesPage } from "../../bridge/reads";
import { es } from "../../i18n/es";
import {
  conversationView,
  turnInFlight,
  withNewest,
  withOlder,
  withPendingVoice,
} from "./conversation";
import { useRememberedOpening } from "./first-lines";
import { rememberHandover } from "./handover";

type Reader = NonNullable<DaemonPort["readSessionMessages"]>;

export type FirstPage = "reading" | "failed" | "read";

function firstPage(
  reads: boolean,
  sessionId: string | null,
  answered: { sessionId: string; read: boolean } | null,
): FirstPage {
  if (!reads) return "read";
  if (answered?.sessionId !== sessionId) return "reading";
  return answered.read ? "read" : "failed";
}

/**
 * When the newest page is read again: after each turn that ended, and when
 * asked to (`again`, after a failed read).
 */
function useNewestReads(inFlight: boolean) {
  const [token, setToken] = useState(0);
  const wasInFlight = useRef(inFlight);
  useEffect(() => {
    if (wasInFlight.current && !inFlight) setToken((value) => value + 1);
    wasInFlight.current = inFlight;
  }, [inFlight]);
  const again = useCallback(() => setToken((value) => value + 1), []);
  return [token, again] as const;
}

/**
 * The open conversation, read from the hub a page at a time, with
 * the turn in progress after it and the voice notes still waiting for the
 * hub. The newest page is read when the conversation opens and again when a
 * turn ends; older ones on demand.
 */
export function useConversation(chat: AppSnapshot["chat"], read?: Reader) {
  const sessionId = chat.session_id;
  const [stored, setStored] = useState<ChatMessage[]>([]);
  const [older, setOlder] = useState<AssistantCursor | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newestToken, readNewestAgain] = useNewestReads(turnInFlight(chat));
  // The conversation whose newest page answered, and whether it was read:
  // until then nothing is known about it, which is not «no messages».
  const [answered, setAnswered] = useState<{ sessionId: string; read: boolean } | null>(null);

  useEffect(() => {
    if (!read || sessionId === null) return;
    let current = true;
    void read(sessionId, null)
      .then((answer: MessagesPage) => {
        if (!current) return;
        setAnswered({ sessionId, read: answer.state === "present" });
        // A failure says so, without its cause; so does an answer that says
        // nothing, which leaves the conversation unread, not empty.
        if (answer.state !== "present") {
          setError(es.assistant.historyFailed);
          return;
        }
        setError(null);
        rememberHandover(sessionId, answer.value.handed_over_from?.agent_id);
        setStored((known) => withNewest(known, answer.value.items));
        setOlder((cursor) => (newestToken === 0 ? answer.value.next : cursor));
      })
      .catch(() => {
        if (!current) return;
        setAnswered({ sessionId, read: false });
        setError(es.assistant.historyFailed);
      });
    return () => {
      current = false;
    };
  }, [newestToken, read, sessionId, chat.history_revision]);

  const loadOlder = useCallback(() => {
    if (!read || sessionId === null || older === null || loading) return;
    setLoading(true);
    setError(null);
    void read(sessionId, older)
      .then((answer) => {
        if (answer.state === "absent") setError(es.assistant.historyFailed);
        if (answer.state !== "present") return;
        setStored((known) => withOlder(known, answer.value.items));
        setOlder(answer.value.next);
      })
      .catch(() => setError(es.assistant.historyFailed))
      .finally(() => setLoading(false));
  }, [loading, older, read, sessionId]);

  const messages = conversationView(stored, chat.turn, turnInFlight(chat));
  const first = firstPage(read !== undefined && sessionId !== null, sessionId, answered);
  // With no older page, the thread holds the conversation's start.
  useRememberedOpening(sessionId, first === "read" && older === null ? messages : null);

  return {
    // The voice notes this Mac keeps for the hub follow the conversation;
    // they are not part of its opening.
    messages: withPendingVoice(messages, chat.pending_voice ?? []),
    hasOlder: older !== null,
    loadOlder,
    loading,
    /**
     * Whether this conversation's first page is known: still being read, its
     * read failed (the error says so), or read. A new conversation, with
     * nothing to read, is read.
     */
    firstPage: first,
    error,
    /** Reads the newest page again, after a failed read. */
    retry: readNewestAgain,
  };
}
