import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { AppCommand } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { Button } from "../../design/Button";
import { Spinner } from "../../design/Spinner";

import { List, Row } from "./layout";
import styles from "./SettingsView.module.css";

type Send = (command: AppCommand) => unknown;
type Settings = AppSnapshot["settings"];

/** What the last search for an update says, and its colour. */
function updateLine(settings: Settings): { text: string; tone?: "ok" | "warn" } {
  const update = settings.update;
  switch (update.state) {
    case "disabled":
      return { text: getCopy().settings.updatesDisabled };
    case "checking":
      return { text: getCopy().settings.checking };
    case "installing":
      return { text: getCopy().settings.installing };
    case "available":
      return { text: getCopy().settings.available(update.version), tone: "ok" };
    case "up_to_date":
    case "not_newer":
      return { text: getCopy().settings.upToDate, tone: "ok" };
    case "failed":
      return { text: getCopy().settings.updateFailed, tone: "warn" };
    case "idle":
      return { text: getCopy().settings.updatesIdle };
  }
}

/** «Versión 0.10.0», where the last search stands, and «Buscar actualización». */
export function Updates({ send, settings }: { send: Send; settings: Settings }) {
  const base = useCopy();
  const update = settings.update;
  const checking = update.state === "checking";
  const line = updateLine(settings);
  const lastFailed = update.last_installation != null && update.last_installation.failure !== null;
  return (
    <List>
      <Row
        aside={
          <div className={styles.actions}>
            {update.state === "available" && (
              <Button
                disabled={Boolean(update.install_blocked)}
                onClick={() => void send({ command: "update_install" })}
                size={32}
                variant="accent"
              >
                {base.settings.installRestart}
              </Button>
            )}
            <Button
              disabled={checking || update.state === "installing" || update.state === "disabled"}
              onClick={() => void send({ command: "update_check" })}
              size={32}
            >
              {checking && <Spinner size={12} />}
              {base.settings.searchUpdate}
            </Button>
          </div>
        }
        help={
          <span aria-live="polite" data-tone={line.tone}>
            {line.text}
          </span>
        }
        title={base.settings.version(settings.versions.app)}
      >
        {lastFailed && <div className={styles.rowHelp}>{base.settings.lastInstallFailed}</div>}
        {Boolean(update.install_blocked) && (
          <div className={styles.rowHelp}>{update.install_blocked}</div>
        )}
      </Row>
    </List>
  );
}
