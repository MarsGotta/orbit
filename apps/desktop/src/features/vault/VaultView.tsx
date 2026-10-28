import { VaultWarnings } from "./VaultWarnings";
import { useCopy } from "../../i18n/CopyProvider";
import type { VaultCatalog } from "../../bridge/generated/VaultCatalog";
import type { VaultSettings } from "../../bridge/generated/VaultSettings";
import type { VaultRoute } from "./route";
import { useState } from "react";
import { TopBarLeading } from "../../app/shell-slots";
import { Button } from "../../design/Button";

import { readFilter, type VaultPort } from "./api";
import { ConnectionCard } from "./ConnectionCard";
import { useVaultConfiguration, useVaultRead } from "./use-vault-read";
import { useVaultRoute } from "./route";
import { VaultColumn } from "./VaultColumn";
import { VaultHome } from "./VaultHome";
import { VaultMap } from "./VaultMap";
import { VaultNotePane } from "./VaultNotePane";
import { VaultTasks } from "./VaultTasks";
import styles from "./VaultReading.module.css";

function RouteContent({
  route,
  catalog,
  configuration,
  port,
  owner,
}: {
  route: VaultRoute;
  catalog: VaultCatalog;
  configuration: VaultSettings;
  port?: VaultPort;
  owner?: string;
}) {
  return (
    <>
      {route.section !== "note" && (
        <VaultWarnings
          className={styles.content}
          warnings={catalog.schema.warnings}
          diagnostics={catalog.schema.diagnostics}
        />
      )}
      {route.section === "home" && (
        <VaultHome catalog={catalog} configuration={configuration} port={port} />
      )}
      {route.section === "note" && (
        <VaultNotePane catalog={catalog} key={route.path} port={port} route={route} />
      )}
      {route.section === "tasks" && <VaultTasks notes={catalog.notes} owner={owner} port={port} />}
      {route.section === "map" && <VaultMap notes={catalog.notes} />}
    </>
  );
}

function IndexPages({
  catalog,
  number,
  setNumber,
}: {
  catalog: VaultCatalog;
  number: number;
  setNumber: (value: number) => void;
}) {
  return (
    <div className={styles.pagination}>
      <span>
        {catalog.notes.length} notas en esta página · {catalog.notes_count + catalog.sources_count}{" "}
        en el vault
      </span>
      {number > 1 && (
        <Button onClick={() => setNumber(number - 1)} size={28}>
          Página anterior del índice
        </Button>
      )}
      {catalog.next_page !== null && (
        <Button onClick={() => setNumber(catalog.next_page ?? 1)} size={28}>
          Más notas del índice
        </Button>
      )}
    </div>
  );
}

function Reading({
  port,
  configuration,
  owner = "",
}: {
  port?: VaultPort;
  configuration: VaultSettings;
  owner?: string;
}) {
  const copy = useCopy().extras.vaultCopy;
  const route = useVaultRoute();
  const [number, setNumber] = useState(1);
  const { answer, error, retry } = useVaultRead(port, readFilter("catalog", "", number));
  const catalog = answer?.outcome === "catalog" ? answer.catalog : null;
  return (
    <div className={styles.view}>
      <VaultColumn catalog={catalog} configuration={configuration} route={route} />
      <div className={styles.reading}>
        {route.section !== "note" && (
          <TopBarLeading>
            <span>
              {route.section === "tasks"
                ? copy.tasks
                : route.section === "map"
                  ? copy.map
                  : copy.home}
            </span>
          </TopBarLeading>
        )}
        {error !== null && (
          <p className={styles.content} role="alert">
            {error}{" "}
            <Button onClick={retry} size={28}>
              Reintentar
            </Button>
          </p>
        )}
        {catalog === null && error === null && (
          <p className={styles.content} role="status">
            Leyendo índice…
          </p>
        )}
        {catalog !== null && (
          <>
            <RouteContent
              route={route}
              catalog={catalog}
              configuration={configuration}
              port={port}
              owner={owner}
            />
            <IndexPages catalog={catalog} number={number} setNumber={setNumber} />
          </>
        )}
      </div>
    </div>
  );
}

export default function VaultView({ port, owner }: { port?: VaultPort; owner?: string }) {
  const copy = useCopy().extras.vaultCopy;
  const { configuration, setConfiguration, error, retry } = useVaultConfiguration(port);
  const route = useVaultRoute();
  if (configuration?.configured === true)
    return <Reading configuration={configuration} owner={owner} port={port} />;
  return (
    <section aria-label="Notas" className={styles.view}>
      {configuration !== null && (
        <VaultColumn catalog={null} configuration={configuration} route={route} />
      )}
      <div className={styles.create}>
        <TopBarLeading>Crear tu vault</TopBarLeading>
        <h1>{copy.createTitle}</h1>
        <p>{copy.createLead}</p>
        {configuration === null && error === null && <p role="status">Leyendo conexión…</p>}
        {error !== null && (
          <p role="alert">
            {error}{" "}
            <Button onClick={retry} size={28}>
              Reintentar
            </Button>
          </p>
        )}
        {configuration !== null && (
          <>
            <h2>1 · Dónde</h2>
            <ConnectionCard
              configuration={configuration}
              key={JSON.stringify(configuration)}
              onSaved={setConfiguration}
              port={port}
            />
            <p className={styles.muted}>{copy.format}</p>
            <p>{copy.readOnly}</p>
          </>
        )}
      </div>
    </section>
  );
}
