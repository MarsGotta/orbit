import { FormatSettings } from "./VaultFormatSettings";
import { LaterSettings } from "./VaultLaterSettings";
import { Button } from "../../design/Button";
import { vaultCopy as copy } from "../../i18n/es/vault";
import { SectionHeading } from "../settings/layout";
import type { VaultPort } from "./api";
import { ConnectionCard } from "./ConnectionCard";
import { useVaultConfiguration } from "./use-vault-read";
import styles from "./VaultReading.module.css";

export function VaultSettingsView({ port }: { port?: VaultPort }) {
  const { configuration, setConfiguration, error, retry } = useVaultConfiguration(port);
  return (
    <>
      <SectionHeading lead={copy.settingsIntro} title="Notas" />
      {error !== null && (
        <p role="alert">
          {error}{" "}
          <Button onClick={retry} size={28}>
            Reintentar
          </Button>
        </p>
      )}
      {configuration === null && error === null && <p role="status">Leyendo conexión…</p>}
      {configuration !== null && (
        <>
          <details className={styles.connection} open={!configuration.configured}>
            <summary>
              {configuration.configured ? "Cambiar conexión del vault" : "Conectar tu vault"}
            </summary>
            <ConnectionCard
              configuration={configuration}
              key={JSON.stringify(configuration)}
              onSaved={setConfiguration}
              port={port}
            />
          </details>
          {configuration.configured && (
            <FormatSettings
              configuration={configuration}
              key={JSON.stringify(configuration)}
              port={port}
              saved={setConfiguration}
            />
          )}
        </>
      )}
      <LaterSettings />
    </>
  );
}
