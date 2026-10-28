import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { VaultConnection } from "../../bridge/generated/VaultConnection";
import type { VaultSettings } from "../../bridge/generated/VaultSettings";
import { Button } from "../../design/Button";
import { Field } from "../../design/Field";
import { Group } from "../settings/layout";
import { message, request, type VaultPort } from "./api";
import styles from "./Vault.module.css";

interface Props {
  configuration: VaultSettings;
  port?: VaultPort;
  onSaved: (value: VaultSettings) => void;
}

function SyncFields({
  draft,
  setDraft,
  busy,
}: {
  draft: VaultConnection;
  setDraft: (value: VaultConnection) => void;
  busy: boolean;
}) {
  const inlineCopy = useCopy().vaultSettings;
  return (
    <>
      <Field
        disabled={busy}
        help={inlineCopy.serverUrlHelp}
        label={inlineCopy.syncUrl}
        onChange={(e) => setDraft({ ...draft, url: e.target.value })}
        placeholder={inlineCopy.syncPlaceholder}
        value={draft.url}
      />
      <Field
        disabled={busy}
        label={inlineCopy.vaultName}
        onChange={(e) => setDraft({ ...draft, vault: e.target.value })}
        value={draft.vault}
      />
      <Field
        autoComplete="new-password"
        disabled={busy}
        help={inlineCopy.tokenHelp}
        label={inlineCopy.token}
        onChange={(e) => setDraft({ ...draft, token: e.target.value })}
        type="password"
        value={draft.token ?? ""}
      />
    </>
  );
}

function useConnection({ configuration, port, onSaved }: Props) {
  const [draft, setDraft] = useState<VaultConnection>({
    ...configuration,
    kind: configuration.kind || "folder",
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async (disconnect = false) => {
    setBusy(true);
    setError(null);
    const { token, kind, path, url, vault } = draft;
    const connection =
      kind === "folder"
        ? {
            kind,
            path,
            url: "",
            vault: "",
            link_style: draft.link_style ?? "markdown",
            maintenance_model: draft.maintenance_model ?? "",
          }
        : {
            kind,
            path: "",
            url,
            vault,
            link_style: draft.link_style ?? "markdown",
            maintenance_model: draft.maintenance_model ?? "",
            ...(token !== undefined && token !== "" ? { token } : {}),
          };
    setDraft((value) => ({ ...value, token: "" }));
    try {
      const result = await request(
        port,
        disconnect ? { call: "disconnect" } : { call: "configure", configuration: connection },
      );
      if (result.outcome === "config") onSaved(result.configuration);
    } catch (error: unknown) {
      setError(message(error));
    } finally {
      setBusy(false);
    }
  };
  return { draft, setDraft, busy, error, save };
}

export function ConnectionCard(props: Props) {
  const inlineCopy = useCopy().vaultSettings;
  const { configuration } = props;
  const { draft, setDraft, busy, error, save } = useConnection(props);
  return (
    <Group help={inlineCopy.connectionHelp} label={inlineCopy.connection}>
      <form
        aria-busy={busy}
        className={styles.settings}
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <label className={styles.selectLabel}>
          {inlineCopy.connector}
          <select
            disabled={busy}
            onChange={(e) => setDraft({ ...draft, kind: e.target.value, token: "" })}
            value={draft.kind}
          >
            <option value="folder">{inlineCopy.hubFolder}</option>
            <option value="fast_note_sync">{inlineCopy.sync}</option>
          </select>
        </label>
        {draft.kind === "folder" ? (
          <Field
            disabled={busy}
            label={inlineCopy.hubPath}
            onChange={(e) => setDraft({ ...draft, path: e.target.value })}
            placeholder={inlineCopy.pathPlaceholder}
            value={draft.path}
          />
        ) : (
          <SyncFields busy={busy} draft={draft} setDraft={setDraft} />
        )}
        {configuration.has_token && <p>{inlineCopy.tokenStored}</p>}
        <div className={styles.actions}>
          <Button disabled={busy} size={36} type="submit" variant="accent">
            {busy ? inlineCopy.checking : inlineCopy.connect}
          </Button>
          {configuration.configured && (
            <Button disabled={busy} onClick={() => void save(true)} size={36} type="button">
              {inlineCopy.disconnect}
            </Button>
          )}
        </div>
        {error !== null && <p role="alert">{error}</p>}
      </form>
    </Group>
  );
}
