import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AssistantFoldersSnapshot } from "../../bridge/generated/AssistantFoldersSnapshot";
import type { FolderAccess } from "../../bridge/generated/FolderAccess";
import type { FolderView } from "../../bridge/generated/FolderView";
import { Button } from "../../design/Button";
import { es as base } from "../../i18n/es";
import { Group, List, SkeletonRows } from "./layout";
import styles from "./SettingsView.module.css";

const copy = base.folders;

type Send = (command: AppCommand) => unknown;

/** Whether a folder opens, in the words of its cause. */
export function accessText(access: FolderAccess): string {
  switch (access.state) {
    case "accessible":
      return copy.accessible;
    case "unknown":
      return copy.unknown;
    case "unavailable": {
      const cause = access.cause;
      switch (cause.kind) {
        case "bookmark_unresolvable":
          return copy.missing;
        case "not_a_folder":
          return copy.notAFolder;
        case "access_denied":
          return copy.accessDenied;
        case "path_too_long":
          return copy.tooLong(cause.bytes, cause.max_bytes);
        case "bookmark_too_large":
          return copy.bookmarkTooLarge(cause.bytes, cause.max_bytes);
      }
    }
  }
}

/**
 * A folder as the person knows it: its name, and where it is, from the
 * home folder or the disk, «Documentos › Nubaria» or «Disco «Archivo»».
 */
export function folderPlace(path: string): { name: string; place: string } {
  const parts = path.split("/").filter(Boolean);
  const name = parts.at(-1) ?? path;
  let inside = parts.slice(0, -1);
  if (inside[0] === "Users" && inside.length >= 2) inside = inside.slice(2);
  else if (inside[0] === "Volumes" && inside.length >= 2)
    inside = [copy.disk(inside[1] ?? ""), ...inside.slice(2)];
  const shown = inside.map(
    (part) => copy.homeFolders[part as keyof typeof copy.homeFolders] ?? part,
  );
  return { name, place: [...shown, name].join(" › ") };
}

function FolderRow({ folder, send }: { folder: FolderView; send: (command: AppCommand) => void }) {
  const { name, place } = folderPlace(folder.path);
  const state = folder.access.state;
  return (
    <div className={styles.folder} data-warn={state === "unavailable" || undefined}>
      <div className={styles.rowText}>
        <div className={styles.rowTitle}>{name}</div>
        <div className={styles.meta} title={folder.path}>
          {place}
        </div>
      </div>
      <div className={styles.folderState} data-state={state}>
        {accessText(folder.access)}
        {state === "unavailable" && <div>{copy.readdHint}</div>}
      </div>
      <Button
        aria-label={copy.remove(name)}
        onClick={() => send({ command: "assistant_folder_remove", id: folder.id })}
        size={28}
        variant="ghost"
      >
        {copy.removeLabel}
      </Button>
    </div>
  );
}

/** «Añadir carpeta»: «Abriendo…» until the daemon has the order; disabled, it says why. */
function AddFolder({ reason, send }: { reason?: string; send: Send }) {
  const [opening, setOpening] = useState(false);
  return (
    <div className={styles.padded}>
      <Button
        disabledReason={reason}
        feedback={opening ? "pending" : "idle"}
        onClick={() => {
          setOpening(true);
          void Promise.resolve(send({ command: "assistant_folder_add" })).finally(() =>
            setOpening(false),
          );
        }}
        pendingLabel={copy.opening}
        size={32}
      >
        {copy.add}
      </Button>
    </div>
  );
}

/** Why no folder can be added now, if anything stops it. */
function addReason(
  folders: AssistantFoldersSnapshot,
  daemonReady: boolean,
  privacyStopped: boolean,
): string | undefined {
  if (!daemonReady) return copy.notReady;
  if (privacyStopped) return base.settings.stoppedTitle;
  if (folders.state === "unreadable") return copy.unreadable;
  if (folders.max > 0 && folders.folders.length >= folders.max) return copy.full(folders.max);
  return undefined;
}

/**
 * «Carpetas que puede leer · 3 de 8». «Añadir carpeta» opens the macOS panel;
 * the folder chosen there is the assistants' to read, and «Quitar» forgets
 * it. Nothing else asks, expires or switches it off. Each error is said
 * inside its row, with what fixes it.
 */
export function AssistantFolders({
  daemonReady,
  folders,
  privacyStopped,
  send,
}: {
  daemonReady: boolean;
  folders?: AssistantFoldersSnapshot;
  privacyStopped: boolean;
  send: Send;
}) {
  const known = folders?.state === "known";
  const count =
    known && folders.max > 0 ? copy.count(folders.folders.length, folders.max) : undefined;
  return (
    <Group count={count} help={copy.help} label={copy.section}>
      {folders === undefined || folders.state === "unknown" ? (
        <SkeletonRows rows={2} />
      ) : (
        <List>
          {folders.state === "unreadable" && (
            <div className={styles.folder} data-warn>
              <div className={styles.rowText}>
                <div className={styles.rowTitle}>{copy.unreadable}</div>
                {folders.unreadable !== undefined && folders.unreadable !== "" && (
                  <div className={styles.meta}>{folders.unreadable}</div>
                )}
              </div>
            </div>
          )}
          {known && folders.folders.length === 0 && <p className={styles.empty}>{copy.empty}</p>}
          {folders.folders.map((folder) => (
            <FolderRow folder={folder} key={folder.id} send={(command) => void send(command)} />
          ))}
          <AddFolder reason={addReason(folders, daemonReady, privacyStopped)} send={send} />
        </List>
      )}
    </Group>
  );
}
