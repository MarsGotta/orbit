import type { AppCommand } from "../../bridge/generated";
import type { NotesIndexSnapshot } from "../../bridge/generated/NotesIndexSnapshot";
import { es } from "../../i18n/es";
import { NotesRefreshButton } from "./NotesRefreshButton";
import styles from "./NotesView.module.css";

/**
 * The list while the index is not read: on its way, or failed with what to
 * do. Counts and causes stay in the log.
 */
export function NotesIndexMessage({
  index,
  send,
}: {
  index: NotesIndexSnapshot;
  send: (command: AppCommand) => Promise<void>;
}) {
  return (
    <section className={styles.empty}>
      <header className={styles.listHeader}>
        <h1>{es.shell.notes}</h1>
        <NotesRefreshButton send={send} />
      </header>
      <NotesIndexStatus index={index} />
    </section>
  );
}

export function NotesIndexStatus({ index }: { index?: NotesIndexSnapshot }) {
  if (index?.state === "loading") return <p role="status">{es.notes.loadingIndex}</p>;
  if (index?.state === "failed") return <p role="alert">{es.notes.indexFailed}</p>;
  if (index?.state === "needs_hub") return <p role="status">{es.notes.needsHub}</p>;
  return null;
}
