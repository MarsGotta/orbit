import { useState } from "react";
import type { VaultCatalog } from "../../bridge/generated/VaultCatalog";
import type { VaultSettings } from "../../bridge/generated/VaultSettings";
import { Button } from "../../design/Button";
import { Field } from "../../design/Field";
import { vaultCopy as copy } from "../../i18n/es/vault";
import { readFilter, type VaultPort } from "./api";
import { noteRoute } from "./route";
import { ProjectCards, RecentActivity } from "./HomeCards";
import { localToday, TaskRows, taskTitle } from "./TaskRows";
import { useVaultRead } from "./use-vault-read";
import styles from "./VaultReading.module.css";

function Search({ port, catalog }: { port?: VaultPort; catalog: VaultCatalog }) {
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [number, setNumber] = useState(1);
  const { answer, error } = useVaultRead(
    port,
    query === "" ? null : readFilter("search", "", number, query),
  );
  const page = answer?.outcome === "page" ? answer.page : null;
  return (
    <div>
      <form
        className={styles.search}
        onSubmit={(event) => {
          event.preventDefault();
          setQuery(draft.trim());
          setNumber(1);
        }}
      >
        <Field
          label={copy.search}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={copy.search}
          type="search"
          value={draft}
        />
        <Button size={36} type="submit">
          Buscar
        </Button>
      </form>
      {query !== "" && (
        <SearchResults
          page={page}
          error={error}
          catalog={catalog}
          number={number}
          setNumber={setNumber}
          close={() => {
            setDraft("");
            setQuery("");
          }}
        />
      )}
    </div>
  );
}

function SearchResults({
  page,
  error,
  catalog,
  number,
  setNumber,
  close,
}: {
  page: import("../../bridge/generated/VaultPage").VaultPage | null;
  error: string | null;
  catalog: VaultCatalog;
  number: number;
  setNumber: (value: number) => void;
  close: () => void;
}) {
  return (
    <section>
      <h2>Resultados</h2>
      {error !== null && <p role="alert">{error}</p>}
      {page === null && error === null && <p role="status">Buscando…</p>}
      <ul>
        {page?.entries.map((entry) => (
          <li key={entry.path}>
            <a href={noteRoute(entry.path)}>
              {entry.title ?? taskTitle(entry.path, catalog.notes)}
            </a>
          </li>
        ))}
      </ul>
      {page?.entries.length === 0 && <p>No se encontraron notas.</p>}
      <div className={styles.actions}>
        {number > 1 && (
          <Button onClick={() => setNumber(number - 1)} size={28}>
            Anterior
          </Button>
        )}
        {page?.next_page != null && (
          <Button onClick={() => setNumber(page.next_page ?? 1)} size={28}>
            Siguiente
          </Button>
        )}
        <Button onClick={close} size={28}>
          Cerrar búsqueda
        </Button>
      </div>
    </section>
  );
}

function DueTasks({ port, catalog }: { port?: VaultPort; catalog: VaultCatalog }) {
  const { answer, error } = useVaultRead(port, { ...readFilter("tasks"), state: "open" });
  const start = localToday();
  const end = new Date(`${start}T12:00:00`);
  end.setDate(end.getDate() + (7 - (end.getDay() || 7)));
  const endText = `${end.getFullYear()}-${String(end.getMonth() + 1).padStart(2, "0")}-${String(end.getDate()).padStart(2, "0")}`;
  const tasks =
    answer?.outcome === "tasks"
      ? answer.tasks.tasks.filter(
          (task) => task.due !== "" && task.due >= start && task.due <= endText,
        )
      : [];
  return (
    <section>
      <h2>
        {copy.due}{" "}
        <a className={styles.minorLink} href="#/vault/tasks">
          {copy.allTasks}
        </a>
      </h2>
      {error !== null && <p role="alert">{error}</p>}
      {answer === null && error === null && <p role="status">Leyendo tareas…</p>}
      {answer !== null && tasks.length === 0 && (
        <p className={styles.muted}>No hay tareas que venzan esta semana en esta página.</p>
      )}
      <TaskRows notes={catalog.notes} tasks={tasks} />
    </section>
  );
}

export function VaultHome({
  port,
  catalog,
  configuration,
}: {
  port?: VaultPort;
  catalog: VaultCatalog;
  configuration: VaultSettings;
}) {
  const name =
    configuration.vault || (configuration.path.split("/").filter(Boolean).pop() ?? "Vault");
  return (
    <div className={styles.content}>
      <header className={styles.heading}>
        <h1>{name}</h1>
        <p>
          {catalog.notes_count} notas y {catalog.sources_count} fuentes en{" "}
          {configuration.kind === "folder" ? configuration.path : "Fast Note Sync"} · OKF{" "}
          {catalog.okf_version || "sin versión declarada"}
        </p>
        <p className={styles.muted}>{copy.readOnly}</p>
      </header>
      <Search catalog={catalog} port={port} />
      <div className={styles.homeGrid}>
        <ProjectCards catalog={catalog} />
        <div>
          <DueTasks catalog={catalog} port={port} />
          <RecentActivity catalog={catalog} />
        </div>
      </div>
    </div>
  );
}
