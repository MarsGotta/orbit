import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { VaultMeta } from "../../bridge/generated/VaultMeta";

import { noteRoute, type VaultRoute } from "./route";
import styles from "./VaultColumn.module.css";

export function VaultFolder({
  folder,
  notes,
  route,
  count,
}: {
  folder: string;
  notes: VaultMeta[];
  route: VaultRoute;
  count: number | null;
}) {
  const copy = useCopy().extras.vaultCopy;
  const [expanded, setExpanded] = useState(
    notes.some((note) => ["person", "project"].includes(note.role)),
  );
  const name = folder.split("/").pop() ?? folder;
  const label = copy.folderNames[name as keyof typeof copy.folderNames] ?? name;
  return (
    <div className={styles.folder}>
      <button
        aria-expanded={expanded}
        className={styles.row}
        onClick={() => setExpanded(!expanded)}
        type="button"
      >
        <span aria-hidden="true">{expanded ? "⌄" : "›"}</span>
        <span className={styles.dot} data-role={notes[0]?.role ?? "note"} />
        <span>{label}</span>
        <span className={styles.count}>{count ?? "…"}</span>
      </button>
      {expanded && <FolderContents folder={folder} notes={notes} route={route} />}
    </div>
  );
}

function FolderContents({
  folder,
  notes,
  route,
}: {
  folder: string;
  notes: VaultMeta[];
  route: VaultRoute;
}) {
  const children = [
    ...new Set(
      notes
        .map(
          (note) =>
            note.path
              .slice(folder.length + 1)
              .split("/")
              .slice(0, -1)[0],
        )
        .filter((name): name is string => name !== undefined),
    ),
  ];
  return (
    <>
      {children.map((name) => {
        const path = `${folder}/${name}`;
        const rows = notes.filter((note) => note.path.startsWith(path + "/"));
        return (
          <div className={styles.nested} key={path}>
            <VaultFolder folder={path} notes={rows} route={route} count={rows.length} />
          </div>
        );
      })}
      {notes
        .filter((note) => note.path.slice(folder.length + 1).indexOf("/") < 0)
        .map((note) => (
          <a
            aria-current={route.path === note.path ? "page" : undefined}
            className={styles.note}
            href={noteRoute(note.path)}
            key={note.path}
          >
            {note.title}
          </a>
        ))}
    </>
  );
}
