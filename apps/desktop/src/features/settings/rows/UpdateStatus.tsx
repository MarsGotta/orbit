import type { SettingsSnapshot } from "../../../bridge/generated";
import { es } from "../../../i18n/es";

/**
 * Where a search for an update the person asked for stands (SIM-24): on its
 * way, its result, or that it failed. The cause, the files and the sizes
 * stay in the log.
 */
export function UpdateStatus({ settings }: { settings: SettingsSnapshot }) {
  const update = settings.update;
  switch (update.state) {
    case "disabled":
      return <p>{es.settings.updatesDisabled}</p>;
    case "checking":
      return <p>{es.settings.checking}</p>;
    case "installing":
      return <p>{es.settings.installing}</p>;
    case "available":
      return <p>{es.settings.available(update.version)}</p>;
    case "up_to_date":
    case "not_newer":
      return <p>{es.settings.upToDate}</p>;
    case "failed":
      return <p>{es.settings.updateFailed}</p>;
    case "idle":
      return null;
  }
}

/** The last installation, only when it did not finish: what to do about it. */
export function LastInstallation({ settings }: { settings: SettingsSnapshot }) {
  const result = settings.update.last_installation;
  if (!result || result.failure === null) return null;
  return <p>{es.settings.lastInstallFailed}</p>;
}
