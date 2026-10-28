import { useCopy } from "../../i18n/CopyProvider";
import { FormatSettings } from "./VaultFormatSettings";
import { LaterSettings } from "./VaultLaterSettings";
import { Button } from "../../design/Button";

import { SectionHeading } from "../settings/layout";
import type { VaultPort } from "./api";
import { ConnectionCard } from "./ConnectionCard";
import { useVaultConfiguration } from "./use-vault-read";
import styles from "./VaultReading.module.css";

export function VaultSettingsView({ port }: { port?: VaultPort }) {
  const inlineCopy = useCopy().vaultSettings;
  const copy = useCopy().extras.vaultCopy;
  const { configuration, setConfiguration, error, retry } = useVaultConfiguration(port);
  return (
    <>
      <SectionHeading lead={copy.settingsIntro} title={inlineCopy.notes} />
      {error !== null && (
        <p role="alert">
          {error}{" "}
          <Button onClick={retry} size={28}>
            {inlineCopy.retry}
          </Button>
        </p>
      )}
      {configuration === null && error === null && (
        <p role="status">{inlineCopy.readingConnection}</p>
      )}
      {configuration !== null && (
        <>
          <details className={styles.connection} open={!configuration.configured}>
            <summary>
              {configuration.configured ? inlineCopy.changeConnection : inlineCopy.connectVault}
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
