import { useCopy } from "../../i18n/CopyProvider";
import type { VaultCatalog } from "../../bridge/generated/VaultCatalog";

import { noteRoute } from "./route";
import styles from "./VaultReading.module.css";
export function ProjectCards({ catalog }: { catalog: VaultCatalog }) {
  const inlineCopy = useCopy().vaultReading;
  const copy = useCopy().extras.vaultCopy;
  return (
    <section>
      <h2>{copy.projects}</h2>
      <div className={styles.projects}>
        {catalog.notes
          .filter((note) => note.role === "project")
          .map((note) => (
            <a className={styles.project} href={noteRoute(note.path)} key={note.path}>
              <div>
                <strong>{note.title}</strong>
                <span className={styles.state} data-state={note.state}>
                  {note.state}
                </span>
              </div>
              <p>{note.description}</p>
              <small>
                {note.start !== "" ? inlineCopy.projectStartAt(note.start) : ""}
                {inlineCopy.taskCount(note.task_count)}
              </small>
            </a>
          ))}
      </div>
      {!catalog.notes.some((note) => note.role === "project") && (
        <p className={styles.muted}>{inlineCopy.noProjects}</p>
      )}
    </section>
  );
}
export function RecentActivity({ catalog }: { catalog: VaultCatalog }) {
  const inlineCopy = useCopy().vaultReading;
  const copy = useCopy().extras.vaultCopy;
  return (
    <section>
      <h2>{copy.recent}</h2>
      <ul className={styles.activity}>
        {catalog.recent.map((event, i) => (
          <li key={i}>
            <span aria-hidden="true" className={styles.avatar}>
              {event.by.charAt(0).toUpperCase()}
            </span>
            <div>
              {event.text !== "" ? (
                event.text
              ) : (
                <>
                  {event.by} · {event.action} ·{" "}
                  {event.path === "" ? (
                    event.title
                  ) : (
                    <a href={noteRoute(event.path)}>{event.title}</a>
                  )}
                </>
              )}
              <small>
                {event.date}
                {event.time !== "" ? ` · ${event.time}` : ""}
                {event.detail === "" ? "" : ` · ${event.detail}`}
              </small>
            </div>
          </li>
        ))}
      </ul>
      {catalog.recent.length === 0 && <p className={styles.muted}>{inlineCopy.noActivity}</p>}
    </section>
  );
}
