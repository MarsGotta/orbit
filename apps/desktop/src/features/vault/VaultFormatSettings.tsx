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
  const [feedback, setFeedback] = useState<{ saved: true } | { error: string } | null>(null);
  const save = async () => {
    setBusy(true);
    setFeedback(null);
    const { kind, path, url, vault } = configuration;
    try {
      const result = await request(port, {
        call: "configure",
        configuration: { kind, path, url, vault, link_style: linkStyle, maintenance_model: model },
      });
      if (result.outcome === "config") {
        saved(result.configuration);
        setFeedback({ saved: true });
      }
    } catch (error: unknown) {
      setFeedback({ error: message(error) });
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
  const inlineCopy = useCopy().vaultSettings;
  const copy = useCopy().extras.vaultCopy;
  const { linkStyle, setLinkStyle, model, busy, feedback, save } = useFormat({
    configuration,
    port,
    saved,
  });
  return (
    <>
      <Group label={inlineCopy.vault}>
        <List>
          <Row
            help={
              configuration.kind === "folder"
                ? configuration.path
                : `${configuration.url} · ${configuration.vault}`
            }
            title={inlineCopy.workingCopy}
          >
            <span>
              {inlineCopy.readOnlyConnector}{" "}
              {configuration.kind === "folder" ? inlineCopy.hubFolder : inlineCopy.sync}
            </span>
          </Row>
          <Row help={copy.formatHelp} title={inlineCopy.format} />
          <Row help={copy.linkHelp} title={inlineCopy.noteLinks}>
            <LinkOptions busy={busy} linkStyle={linkStyle} setLinkStyle={setLinkStyle} />
          </Row>
        </List>
      </Group>
      <Maintainer model={model} />
      <div className={styles.actions}>
        <Button disabled={busy} onClick={() => void save()} size={36} variant="accent">
          {busy ? inlineCopy.checking : inlineCopy.save}
        </Button>
        <span role="status">
          {feedback === null ? "" : "saved" in feedback ? inlineCopy.saved : feedback.error}
        </span>
      </div>
    </>
  );
}

function Maintainer({ model }: { model: string }) {
  const inlineCopy = useCopy().vaultSettings;
  const copy = useCopy().extras.vaultCopy;
  return (
    <Group help={inlineCopy.maintainerHelp} label={inlineCopy.maintainer}>
      <List>
        <Row title={inlineCopy.hubJob}>
          <p>{model || inlineCopy.noModel}</p>
          <a href="#/settings/ia">{inlineCopy.chooseModel}</a>
        </Row>
        <Row help={copy.schemaHelp} title={inlineCopy.schema}>
          <a href={noteRoute("SCHEMA.md")}>{inlineCopy.readSchema}</a>
        </Row>
        <Row help={copy.untouched} title={inlineCopy.untouched} />
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
  const inlineCopy = useCopy().vaultSettings;
  return (
    <fieldset className={styles.linkOptions} disabled={busy}>
      <legend className={styles.visuallyHidden}>{inlineCopy.linkStyle}</legend>
      {(
        [
          ["markdown", inlineCopy.markdown],
          ["wikilink", inlineCopy.wikilinks],
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
