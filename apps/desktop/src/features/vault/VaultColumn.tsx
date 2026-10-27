import type { VaultCatalog } from "../../bridge/generated/VaultCatalog";
import type { VaultSettings } from "../../bridge/generated/VaultSettings";
import { ListColumn, useShellSlots } from "../../app/shell-slots";
import { vaultCopy as copy } from "../../i18n/es/vault";
import { noteRoute, type VaultRoute } from "./route";
import styles from "./VaultColumn.module.css";
import { VaultFolder as Folder } from "./VaultFolder";

const ORDER = Object.keys(copy.folderNames);

function Folders({ catalog, route }: { catalog: VaultCatalog | null; route: VaultRoute }) {
  const notes = catalog?.notes ?? [];
  const groups = [...new Set([...ORDER, ...notes.map((note) => note.path.split("/")[0] ?? "")])];
  return (
    <>
      {groups
        .filter((group) => !group.endsWith(".md"))
        .map((folder) => {
          const rows = notes.filter((note) => note.path.startsWith(folder + "/"));
          const type =
            Object.entries(catalog?.schema.folders ?? {}).find(
              ([, path]) => path === folder + "/",
            )?.[0] ??
            rows[0]?.type ??
            "";
          return (
            <Folder
              count={catalog === null ? null : (catalog.type_counts[type] ?? rows.length)}
              folder={folder}
              key={folder}
              notes={rows}
              route={route}
            />
          );
        })}
      {notes
        .filter((note) => !note.path.includes("/"))
        .map((note) => (
          <a className={styles.note} href={noteRoute(note.path)} key={note.path}>
            {note.title}
          </a>
        ))}
    </>
  );
}

function VaultIdentity({ name, catalog }: { name: string; catalog: VaultCatalog | null }) {
  return (
    <div className={styles.identity}>
      <span className={styles.initial}>{name.charAt(0).toUpperCase()}</span>
      <div>
        <strong>{name}</strong>
        <small>
          {catalog === null
            ? "Leyendo índice…"
            : `${catalog.notes_count} notas · ${catalog.sources_count} fuentes`}
        </small>
      </div>
    </div>
  );
}

function Contents({
  catalog,
  configuration,
  route,
}: {
  catalog: VaultCatalog | null;
  configuration: VaultSettings;
  route: VaultRoute;
}) {
  if (!configuration.configured) return <p className={styles.empty}>{copy.emptyAside}</p>;
  const name =
    configuration.vault || (configuration.path.split("/").filter(Boolean).pop() ?? "Vault");
  return (
    <>
      <VaultIdentity name={name} catalog={catalog} />
      <nav aria-label="Secciones de Notas">
        {(
          [
            ["home", "Inicio", "#/vault"],
            ["tasks", "Tareas", "#/vault/tasks"],
            ["map", "Mapa", "#/vault/map"],
          ] as const
        ).map(([id, label, href]) => (
          <a
            aria-current={route.section === id ? "page" : undefined}
            className={styles.row}
            href={href}
            key={id}
          >
            {label}
            {id === "tasks" && catalog !== null && (
              <span aria-label={`${catalog.open_tasks} tareas abiertas`} className={styles.count}>
                {catalog.open_tasks}
              </span>
            )}
          </a>
        ))}
      </nav>
      <p className={styles.caption}>{copy.folders}</p>
      <Folders catalog={catalog} route={route} />
      <footer className={styles.footer}>
        <a href={noteRoute("SCHEMA.md")}>
          Esquema <code>SCHEMA.md</code>
        </a>
        <a href={noteRoute("index.md")}>
          Índice <code>index.md</code>
        </a>
        <a href="#/settings/notas">Ajustes de Notas</a>
        <small>
          {catalog === null
            ? "Índice pendiente"
            : `OKF ${catalog.okf_version || "sin versión declarada"} · lectura desde el hub`}
        </small>
      </footer>
    </>
  );
}

export function VaultColumn(props: {
  catalog: VaultCatalog | null;
  configuration: VaultSettings;
  route: VaultRoute;
}) {
  const slots = useShellSlots();
  const contents = (
    <div className={styles.contents}>
      <Contents {...props} />
    </div>
  );
  if (slots === null)
    return (
      <aside aria-label={copy.aside} className={styles.column}>
        <h2>{copy.title}</h2>
        {contents}
      </aside>
    );
  return (
    <ListColumn label={copy.aside} title={copy.title}>
      {contents}
    </ListColumn>
  );
}
