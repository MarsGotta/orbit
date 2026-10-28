import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { VaultSettings } from "../../bridge/generated/VaultSettings";
import { Button } from "../../design/Button";

import { Group, List, Row } from "../settings/layout";
import { message, request, type VaultPort } from "./api";
import { noteRoute } from "./route";
import styles from "./VaultReading.module.css";
function useFormat({
  configuration,
  port,
  saved,
}: {
  configuration: VaultSettings;
  port?: VaultPort;
  saved: (value: VaultSettings) => void;
}) {
  const [linkStyle, setLinkStyle] = useState(configuration.link_style);
  const model = configuration.maintenance_model;
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState("");
  const save = async () => {
    setBusy(true);
    setFeedback("");
    const { kind, path, url, vault } = configuration;
    try {
      const result = await request(port, {
        call: "configure",
        configuration: { kind, path, url, vault, link_style: linkStyle, maintenance_model: model },
      });
      if (result.outcome === "config") {
        saved(result.configuration);
        setFeedback("Guardado en el hub.");
      }
    } catch (error: unknown) {
      setFeedback(message(error));
    } finally {
      setBusy(false);
    }
  };
  return { linkStyle, setLinkStyle, model, busy, feedback, save };
}
export function FormatSettings({
  configuration,
  port,
  saved,
}: {
  configuration: VaultSettings;
  port?: VaultPort;
  saved: (value: VaultSettings) => void;
}) {
  const copy = useCopy().extras.vaultCopy;
  const { linkStyle, setLinkStyle, model, busy, feedback, save } = useFormat({
    configuration,
    port,
    saved,
  });
  return (
    <>
      <Group label="Vault">
        <List>
          <Row
            help={
              configuration.kind === "folder"
                ? configuration.path
                : `${configuration.url} · ${configuration.vault}`
            }
            title="Copia de trabajo en el hub"
          >
            <span>
              Conector de solo lectura:{" "}
              {configuration.kind === "folder" ? "Carpeta en el hub" : "Fast Note Sync"}
            </span>
          </Row>
          <Row help={copy.formatHelp} title="Formato" />
          <Row help={copy.linkHelp} title="Enlaces entre notas">
            <LinkOptions busy={busy} linkStyle={linkStyle} setLinkStyle={setLinkStyle} />
          </Row>
        </List>
      </Group>
      <Maintainer model={model} />
      <div className={styles.actions}>
        <Button disabled={busy} onClick={() => void save()} size={36} variant="accent">
          {busy ? "Comprobando…" : "Guardar configuración"}
        </Button>
        <span role="status">{feedback}</span>
      </div>
    </>
  );
}

function Maintainer({ model }: { model: string }) {
  const copy = useCopy().extras.vaultCopy;
  return (
    <Group
      help="Un trabajo del hub con su propio modelo. No se ejecuta en esta fase."
      label="Quién lo mantiene"
    >
      <List>
        <Row title="Trabajo del hub">
          <p>{model || "Sin modelo configurado"}</p>
          <a href="#/settings/ia">Elegir modelo en Ajustes › IA</a>
        </Row>
        <Row help={copy.schemaHelp} title="Esquema SCHEMA.md">
          <a href={noteRoute("SCHEMA.md")}>Leer el esquema</a>
        </Row>
        <Row help={copy.untouched} title="Lo que no toca" />
      </List>
    </Group>
  );
}

function LinkOptions({
  busy,
  linkStyle,
  setLinkStyle,
}: {
  busy: boolean;
  linkStyle: string;
  setLinkStyle: (value: string) => void;
}) {
  return (
    <fieldset className={styles.linkOptions} disabled={busy}>
      <legend className={styles.visuallyHidden}>Estilo de enlaces</legend>
      {(
        [
          ["markdown", "Markdown (OKF)"],
          ["wikilink", "[[Wikilinks]]"],
        ] as const
      ).map(([value, label]) => (
        <label key={value}>
          <input
            checked={linkStyle === value}
            name="link-style"
            onChange={() => setLinkStyle(value)}
            type="radio"
          />
          {label}
        </label>
      ))}
    </fieldset>
  );
}
