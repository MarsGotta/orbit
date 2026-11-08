import { useState } from "react";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { Button } from "../../design/Button";
import { Skeleton } from "../../design/Skeleton";
import { useCopy } from "../../i18n/CopyProvider";
import { Row } from "./layout";
import styles from "./SettingsView.module.css";

type Settings = AppSnapshot["settings"];

function hostOf(url: string | null): string | null {
  if (url === null) return null;
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/**
 * This Mac and the hub it is paired with. Pairing again a Mac that is paired
 * asks first, inside the page: the hub will see a new device.
 */
export function ThisMac({
  onReopen,
  settings,
  statusKnown,
}: {
  onReopen: (mode?: "pair") => void;
  settings: Settings;
  statusKnown: boolean;
}) {
  const es = useCopy();
  const [confirming, setConfirming] = useState(false);
  const hub = settings.hub;
  const revoked = hub.connection === "revoked";
  const paired = hub.paired && !revoked;
  const host = hostOf(hub.url);
  const help = !statusKnown ? (
    <Skeleton height={10} widths={["40%"]} />
  ) : revoked ? (
    es.settings.revoked
  ) : !hub.paired ? (
    es.settings.unpaired
  ) : host !== null ? (
    es.settings.pairedWith(host)
  ) : undefined;
  return (
    <>
      <Row
        aside={
          !confirming && (
            <Button
              onClick={() => (paired ? setConfirming(true) : onReopen("pair"))}
              size={32}
              variant={paired ? "outline" : "accent"}
            >
              {paired ? es.settings.pairAgain : es.settings.pair}
            </Button>
          )
        }
        help={help}
        title={hub.device_name ?? es.settings.thisMac}
        warn={statusKnown && !paired}
      />
      {confirming && (
        <div className={styles.confirm} role="alert">
          <div className={styles.rowTitle}>{es.settings.pairAgainQuestion}</div>
          <div className={styles.rowHelp}>{es.settings.pairAgainWarning}</div>
          <div className={styles.actions}>
            <Button onClick={() => onReopen("pair")} size={32} variant="accent">
              {es.settings.confirmPairAgain}
            </Button>
            <Button onClick={() => setConfirming(false)} size={32} variant="ghost">
              {es.settings.cancelPairAgain}
            </Button>
          </div>
        </div>
      )}
    </>
  );
}
