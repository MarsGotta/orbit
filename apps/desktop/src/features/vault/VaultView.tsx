import { VaultPageBar } from "./VaultPageBar";
import { VaultOwner, normalizeOwner } from "./provenance";
import { DistillationView } from "./DistillationView";
import { useDistillations } from "./use-distillations";
import { VaultWarnings } from "./VaultWarnings";
import { useCopy } from "orbit-ui/i18n/CopyProvider";
import type { VaultCatalog } from "orbit-ui/bridge/generated/VaultCatalog";
import type { VaultSettings } from "orbit-ui/bridge/generated/VaultSettings";
import type { VaultRoute } from "./route";
import { useState } from "react";
import { TopBarLeading } from "../../app/shell-slots";
import { Button } from "orbit-ui/design/Button";

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
  proposals,
}: {
  route: VaultRoute;
  catalog: VaultCatalog;
  configuration: VaultSettings;
  port?: VaultPort;
  owner?: string;
  proposals: ReturnType<typeof useDistillations>;
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
        <VaultHome
          catalog={catalog}
          configuration={configuration}
          port={port}
          proposals={proposals}
        />
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
  const inlineCopy = useCopy().vaultReading;
  return (
    <div className={styles.pagination}>
      <span>
        {inlineCopy.pageCounts(catalog.notes.length, catalog.notes_count + catalog.sources_count)}
      </span>
      {number > 1 && (
        <Button onClick={() => setNumber(number - 1)} size={28}>
          {inlineCopy.previousIndex}
        </Button>
      )}
      {catalog.next_page !== null && (
        <Button onClick={() => setNumber(catalog.next_page ?? 1)} size={28}>
          {inlineCopy.moreIndex}
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
  const inlineCopy = useCopy().vaultReading;
  const route = useVaultRoute();
  const proposals = useDistillations(port);
  const [number, setNumber] = useState(1);
  const { answer, error, retry } = useVaultRead(port, readFilter("catalog", "", number));
  const catalog = answer?.outcome === "catalog" ? answer.catalog : null;
  return (
    <VaultOwner value={catalog?.schema.owner ?? normalizeOwner(owner)}>
      <div className={styles.view}>
        <VaultColumn
          catalog={catalog}
          configuration={configuration}
          route={route}
          proposals={proposals}
        />
        <div className={styles.reading}>
          {route.section === "distill" && (
            <DistillationView key={route.path} id={route.path} port={port} />
          )}
          <VaultPageBar route={route} />
          {error !== null && (
            <p className={styles.content} role="alert">
              {error}{" "}
              <Button onClick={retry} size={28}>
                {inlineCopy.retry}
              </Button>
            </p>
          )}
          {catalog === null && error === null && (
            <p className={styles.content} role="status">
              {inlineCopy.readingIndex}
            </p>
          )}
          {catalog !== null && route.section !== "distill" && (
            <>
              <RouteContent
                route={route}
                catalog={catalog}
                configuration={configuration}
                port={port}
                owner={owner}
                proposals={proposals}
              />
              <IndexPages catalog={catalog} number={number} setNumber={setNumber} />
            </>
          )}
        </div>
      </div>
    </VaultOwner>
  );
}

export default function VaultView({ port, owner }: { port?: VaultPort; owner?: string }) {
  const inlineCopy = useCopy().vaultReading;
  const copy = useCopy().extras.vaultCopy;
  const { configuration, setConfiguration, error, retry } = useVaultConfiguration(port);
  const route = useVaultRoute();
  if (configuration?.configured === true)
    return <Reading configuration={configuration} owner={owner} port={port} />;
  return (
    <section aria-label={inlineCopy.notesTitle} className={styles.view}>
      {configuration !== null && (
        <VaultColumn catalog={null} configuration={configuration} route={route} />
      )}
      <div className={styles.create}>
        <TopBarLeading>{inlineCopy.createVault}</TopBarLeading>
        <h1>{copy.createTitle}</h1>
        <p>{copy.createLead}</p>
        {configuration === null && error === null && (
          <p role="status">{inlineCopy.readingConnection}</p>
        )}
        {error !== null && (
          <p role="alert">
            {error}{" "}
            <Button onClick={retry} size={28}>
              {inlineCopy.retry}
            </Button>
          </p>
        )}
        {configuration !== null && (
          <>
            <h2>{inlineCopy.where}</h2>
            <ConnectionCard
              configuration={configuration}
              key={JSON.stringify(configuration)}
              onSaved={setConfiguration}
              port={port}
            />
            <p className={styles.muted}>{copy.format}</p>
          </>
        )}
      </div>
    </section>
  );
}
