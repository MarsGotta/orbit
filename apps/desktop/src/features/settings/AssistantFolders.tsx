import type { AppCommand } from "../../bridge/generated";
import type { AssistantFoldersSnapshot } from "../../bridge/generated/AssistantFoldersSnapshot";
import type { FolderAccess } from "../../bridge/generated/FolderAccess";
import { Button } from "../../design/Button";
import { Card } from "../../design/Card";
import { SectionLabel } from "../../design/SectionLabel";
import { useSpanish } from "../../i18n/AssistantCopy";
import { es as base } from "../../i18n/es";
import styles from "./SettingsView.module.css";

/** Whether a folder opens, in the words of its cause. */
export function accessText(access: FolderAccess, copy = base): string {
  const folders = copy.folders;
  switch (access.state) {
    case "accessible":
      return folders.accessible;
    case "unknown":
      return folders.unknown;
    case "unavailable": {
      const cause = access.cause;
      switch (cause.kind) {
        case "bookmark_unresolvable":
          return folders.missing;
        case "not_a_folder":
          return folders.notAFolder;
        case "access_denied":
          return folders.accessDenied;
        case "path_too_long":
          return folders.tooLong(cause.bytes, cause.max_bytes);
        case "bookmark_too_large":
          return folders.bookmarkTooLarge(cause.bytes, cause.max_bytes);
      }
    }
  }
}

/**
 * Ajustes › Carpetas del asistente. «Añadir carpeta» opens the macOS
 * panel; the folder chosen there is the assistant's to read, and «Quitar»
 * forgets it. Nothing else asks, expires or switches it off.
 */
export function AssistantFolders({
  daemonReady,
  folders,
  send,
}: {
  daemonReady: boolean;
  folders?: AssistantFoldersSnapshot;
  send: (command: AppCommand) => unknown;
}) {
  const es = useSpanish();
  const copy = es.folders;
  const list = folders?.folders ?? [];
  const max = folders?.max ?? 0;
  const full = max > 0 && list.length >= max;
  return (
    <section aria-label={copy.section} className={styles.section}>
      <SectionLabel>{copy.section}</SectionLabel>
      <Card>
        <p>{es.settings.foldersHelp}</p>
        {folders?.state === "unreadable" ? (
          <p>
            <strong>{copy.unreadable}</strong> {folders.unreadable}
          </p>
        ) : null}
        {folders?.state === "known" && list.length === 0 ? <p>{copy.empty}</p> : null}
        <div className={styles.controls}>
          {list.map((folder) => (
            <div className={styles.controlRow} key={folder.id}>
              <div className={styles.rowCopy}>
                <strong>{folder.path}</strong>
                <span>{accessText(folder.access, es)}</span>
              </div>
              <Button
                aria-label={copy.remove(folder.path)}
                onClick={() => void send({ command: "assistant_folder_remove", id: folder.id })}
              >
                {copy.removeLabel}
              </Button>
            </div>
          ))}
        </div>
        {full ? <p>{copy.full(max)}</p> : null}
        <Button
          disabled={!daemonReady || full || folders?.state === "unreadable"}
          onClick={() => void send({ command: "assistant_folder_add" })}
        >
          {copy.add}
        </Button>
      </Card>
    </section>
  );
}
