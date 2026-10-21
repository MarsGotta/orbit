import type { AppView as AppSnapshot } from "../../state/app-view";
import { useCallback, useEffect, useRef, useState } from "react";

import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { DaemonPort } from "../../bridge/port";
import type { AssistantCursor, MessagesPage } from "../../bridge/reads";
import { es } from "../../i18n/es";
import { conversationView, turnInFlight, withNewest, withOlder } from "./conversation";

type Reader = NonNullable<DaemonPort["readSessionMessages"]>;

/**
 * The open conversation, read from the hub a page at a time, with
 * the turn in progress after it. The newest page is read when the
 * conversation opens and again when a turn ends; older ones on demand.
 */
export function useConversation(chat: AppSnapshot["chat"], read?: Reader) {
  const sessionId = chat.session_id;
  const inFlight = turnInFlight(chat);
  const [stored, setStored] = useState<ChatMessage[]>([]);
  const [older, setOlder] = useState<AssistantCursor | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newestToken, setNewestToken] = useState(0);
  const wasInFlight = useRef(inFlight);

  useEffect(() => {
    if (wasInFlight.current && !inFlight) setNewestToken((token) => token + 1);
    wasInFlight.current = inFlight;
  }, [inFlight]);

  useEffect(() => {
    if (!read || sessionId === null) return;
    let current = true;
    void read(sessionId, null)
      .then((answer: MessagesPage) => {
        if (!current) return;
        // A failure says so, without its cause; an unknown is quiet.
        if (answer.state === "absent") setError(es.assistant.historyFailed);
        if (answer.state !== "present") return;
        setError(null);
        setStored((known) => withNewest(known, answer.value.items));
        setOlder((cursor) => (newestToken === 0 ? answer.value.next : cursor));
      })
      .catch(() => {
        if (current) setError(es.assistant.historyFailed);
      });
    return () => {
      current = false;
    };
  }, [newestToken, read, sessionId]);

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

  return {
    messages: conversationView(stored, chat.turn, inFlight),
    hasOlder: older !== null,
    loadOlder,
    loading,
    error,
  };
}
