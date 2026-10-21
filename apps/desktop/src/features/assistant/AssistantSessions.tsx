import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { ContextPanel, contextForView } from "../../app/ContextPanel";
import { es } from "../../i18n/es";
import { turnInFlight } from "./conversation";
import { useAssistantSessions, type SessionsState } from "./useAssistantSessions";

function SessionsFooter({
  state,
  more,
  retry,
}: {
  state: SessionsState;
  more: () => void;
  retry: () => void;
}) {
  return (
    <div aria-live="polite">
      {state.loading && <p>{es.assistant.loadingSessions}</p>}
      {state.error !== null && <p role="alert">{state.error}</p>}
      {state.error !== null && (
        <button onClick={retry} type="button">
          {es.assistant.retrySessions}
        </button>
      )}
      {!state.loading && state.next !== null && (
        <button onClick={more} type="button">
          {es.assistant.loadMoreSessions}
        </button>
      )}
    </div>
  );
}

export function AssistantSessions({
  snapshot,
  send,
  read,
}: {
  snapshot: AppSnapshot;
  send: (command: AppCommand) => void;
  read?: DaemonPort["readSessionsPage"];
}) {
  const { state, more, retry } = useAssistantSessions(
    snapshot.chat.session_id ?? undefined,
    turnInFlight(snapshot.chat),
    read,
  );
  const data = contextForView("assistant", snapshot, send, state.items);
  if (!data) return null;
  const count = String(state.total ?? state.items.length);
  return (
    <ContextPanel
      data={{ ...data, count }}
      footer={<SessionsFooter more={more} retry={retry} state={state} />}
    />
  );
}
