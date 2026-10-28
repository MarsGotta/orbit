import { useCopy } from "../../i18n/CopyProvider";
import type { AppCommand } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { ListColumn } from "../../app/shell-slots";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";
import { Loading } from "../../design/Spinner";

import type { AppView as AppSnapshot } from "../../state/app-view";
import { goTo } from "./assistant-route";
import { Conversations, useRows } from "./ConversationList";
import { turnInFlight } from "./conversation";
import { useAssistantSessions, type SessionsState } from "./useAssistantSessions";
import { YourAssistants } from "./YourAssistants";
import styles from "./Sessions.module.css";

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
  const es = useCopy();
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

/**
 * The window's list column (`ListColumn`) of the Asistente: «+ Nueva»,
 * «Tus asistentes» with «+» to add one, and «Conversaciones» by date, each
 * with its assistant's avatar and name. A row's name keeps when it began
 * («Hoy, 09:31»), as the real-test harness finds it.
 */
export function AssistantSessions({
  snapshot,
  read,
  send,
}: {
  snapshot: AppSnapshot;
  send: (command: AppCommand) => unknown;
  read?: DaemonPort["readSessionsPage"];
}) {
  const assistantsCopy = useCopy().extras.assistantsCopy;
  const es = useCopy();
  const { chat } = snapshot;
  const { state, more, retry } = useAssistantSessions(
    chat.session_id ?? undefined,
    turnInFlight(chat),
    read,
  );
  const rows = useRows(snapshot, state.items);
  return (
    <ListColumn
      actions={
        <Button
          aria-label={es.assistant.newSession}
          className={styles.newButton}
          onClick={() => goTo({ screen: "new" })}
          size={28}
        >
          <Icon name="plus" size={14} />
          {es.assistant.newShort}
        </Button>
      }
      label={assistantsCopy.listLabel}
      title={assistantsCopy.title}
    >
      <YourAssistants snapshot={snapshot} />
      <h3 className={styles.group}>{assistantsCopy.conversations}</h3>
      {!state.loaded && state.error === null ? (
        <ul aria-busy="true" aria-label={es.assistant.loadingSessions} className={styles.list}>
          <SkeletonRows />
        </ul>
      ) : (
        <Conversations rows={rows} send={send} snapshot={snapshot} />
      )}
      <Footer more={more} retry={retry} state={state} />
    </ListColumn>
  );
}
