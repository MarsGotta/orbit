import { NoteFacts } from "./NoteFacts";
import { useState } from "react";
import type { VaultCatalog } from "../../bridge/generated/VaultCatalog";
import type { VaultNote } from "../../bridge/generated/VaultNote";
import { Button } from "../../design/Button";
import { vaultCopy as copy } from "../../i18n/es/vault";
import { readFilter, type VaultPort } from "./api";
import { internalTarget } from "./markdown-tree";
import { noteRoute } from "./route";
import { taskTitle } from "./TaskRows";
import { useVaultRead } from "./use-vault-read";
import styles from "./VaultReading.module.css";

function Sources({ note }: { note: VaultNote }) {
  return (
    <section>
      <h3>
        {copy.sources} · {note.citations.length}
      </h3>
      {note.citations.length === 0 && <p>{copy.noSources}</p>}
      <ol className={styles.sources}>
        {note.citations.map((source) => {
          const internal = internalTarget(note.path, source.resource);
          return (
            <li id={`vault-source-${source.id}`} key={source.id}>
              <span className={styles.citation}>{source.number}</span>
              <div>
                {internal !== null ? (
                  <a href={noteRoute(internal)}>{source.title}</a>
                ) : /^https?:\/\//.test(source.resource) ? (
                  <a href={source.resource} rel="noopener noreferrer" target="_blank">
                    {source.title}
                  </a>
                ) : (
                  <span>{source.title}</span>
                )}
                <small>{source.resource === "" ? "Fuente no declarada" : source.resource}</small>
              </div>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

function Backlinks({
  note,
  catalog,
  port,
}: {
  note: VaultNote;
  catalog: VaultCatalog;
  port?: VaultPort;
}) {
  const [number, setNumber] = useState(1);
  const { answer, error, retry } = useVaultRead(port, readFilter("backlinks", note.path, number));
  const page = answer?.outcome === "page" ? answer.page : null;
  const count = catalog.notes.find((entry) => entry.path === note.path)?.backlink_count;
  return (
    <section>
      <h3>
        {copy.backlinks}
        {count === undefined ? "" : ` · ${count}`}
      </h3>
      {error !== null && (
        <p role="alert">
          {error}{" "}
          <Button onClick={retry} size={28}>
            Reintentar
          </Button>
        </p>
      )}
      {page === null && error === null && <p role="status">Leyendo enlaces…</p>}
      {page?.entries.length === 0 && <p>Ninguna nota enlaza aquí.</p>}
      <ul className={styles.backlinks}>
        {page?.entries.map((entry) => {
          const meta = catalog.notes.find((n) => n.path === entry.path);
          return (
            <li key={entry.path}>
              <a href={noteRoute(entry.path)}>
                {entry.title ?? taskTitle(entry.path, catalog.notes)}
              </a>
              <small>{entry.type ?? meta?.type ?? ""}</small>
              <p>{entry.snippet ?? meta?.description ?? ""}</p>
            </li>
          );
        })}
      </ul>
      <div className={styles.actions}>
        {number > 1 && (
          <Button onClick={() => setNumber(number - 1)} size={28}>
            Anteriores
          </Button>
        )}
        {page?.next_page != null && (
          <Button onClick={() => setNumber(page.next_page ?? 1)} size={28}>
            Más enlaces
          </Button>
        )}
      </div>
    </section>
  );
}

export function AboutNote({
  note,
  catalog,
  port,
}: {
  note: VaultNote;
  catalog: VaultCatalog;
  port?: VaultPort;
}) {
  return (
    <aside aria-label={copy.about} className={styles.about}>
      <NoteFacts note={note} />
      <Sources note={note} />
      <Backlinks catalog={catalog} note={note} port={port} />
      <History note={note} port={port} />
    </aside>
  );
}

function History({ note, port }: { note: VaultNote; port?: VaultPort }) {
  const [number, setNumber] = useState(1);
  const { answer, error } = useVaultRead(
    port,
    number > 1 ? readFilter("read", note.path, number) : null,
  );
  const data = number === 1 ? note : answer?.outcome === "note" ? answer.note : null;
  return (
    <section>
      <h3>{copy.history}</h3>
      {error !== null && <p role="alert">{error}</p>}
      {data === null && error === null && <p role="status">Leyendo historial…</p>}
      {data?.history.length === 0 && <p>{copy.noHistory}</p>}
      {data?.history.map((entry, i) => (
        <p key={i}>
          {entry.text !== "" ? entry.text : `${entry.by} · ${entry.action} · ${entry.detail}`}
          <small>
            {entry.date}
            {entry.time !== "" ? ` · ${entry.time}` : ""}
          </small>
        </p>
      ))}
      <div className={styles.actions}>
        {number > 1 && (
          <Button onClick={() => setNumber(number - 1)} size={28}>
            Historial anterior
          </Button>
        )}
        {data?.history_next_page != null && (
          <Button onClick={() => setNumber(data.history_next_page ?? 1)} size={28}>
            Más historial
          </Button>
        )}
      </div>
    </section>
  );
}
