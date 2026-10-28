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
  return (
    <>
      <Field
        disabled={busy}
        help="URL del servidor, sin /api."
        label="URL de Fast Note Sync"
        onChange={(e) => setDraft({ ...draft, url: e.target.value })}
        placeholder="https://sync.example.test"
        value={draft.url}
      />
      <Field
        disabled={busy}
        label="Nombre del vault"
        onChange={(e) => setDraft({ ...draft, vault: e.target.value })}
        value={draft.vault}
      />
      <Field
        autoComplete="new-password"
        disabled={busy}
        help="Se cifra en el hub. Déjalo vacío para conservar el token de esta conexión."
        label="Token"
        onChange={(e) => setDraft({ ...draft, token: e.target.value })}
        type="password"
        value={draft.token ?? ""}
      />
    </>
  );
}

function useConnection({ configuration, port, onSaved }: Props) {
  useCopy();
  const [draft, setDraft] = useState<VaultConnection>({
    ...configuration,
    kind: configuration.kind || "folder",
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
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
      setError(error);
    } finally {
      setBusy(false);
    }
  };
  return { draft, setDraft, busy, error: error === null ? null : message(error), save };
}

export function ConnectionCard(props: Props) {
  const { configuration } = props;
  const { draft, setDraft, busy, error, save } = useConnection(props);
  return (
    <Group
      help="Orbit lee tu vault en Markdown. La carpeta está en el hub; el Mac no la abre."
      label="Conexión del vault"
    >
      <form
        aria-busy={busy}
        className={styles.settings}
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <label className={styles.selectLabel}>
          Conector
          <select
            disabled={busy}
            onChange={(e) => setDraft({ ...draft, kind: e.target.value, token: "" })}
            value={draft.kind}
          >
            <option value="folder">Carpeta en el hub</option>
            <option value="fast_note_sync">Fast Note Sync</option>
          </select>
        </label>
        {draft.kind === "folder" ? (
          <Field
            disabled={busy}
            label="Ruta en el hub"
            onChange={(e) => setDraft({ ...draft, path: e.target.value })}
            placeholder="/ruta/absoluta/al/vault"
            value={draft.path}
          />
        ) : (
          <SyncFields busy={busy} draft={draft} setDraft={setDraft} />
        )}
        {configuration.has_token && <p>Token guardado en el hub.</p>}
        <div className={styles.actions}>
          <Button disabled={busy} size={36} type="submit" variant="accent">
            {busy ? "Comprobando…" : "Comprobar y conectar"}
          </Button>
          {configuration.configured && (
            <Button disabled={busy} onClick={() => void save(true)} size={36} type="button">
              Desconectar
            </Button>
          )}
        </div>
        {error !== null && <p role="alert">{error}</p>}
      </form>
    </Group>
  );
}
