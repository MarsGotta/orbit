import type { AppView as AppSnapshot } from "../../../state/app-view";
import { useState } from "react";
import type { SettingsSnapshot } from "../../../bridge/generated";
import { Button } from "../../../design/Button";
import { useSpanish } from "../../../i18n/AssistantCopy";
import styles from "../SettingsView.module.css";

type AccountPairingProps = {
  hub: SettingsSnapshot["hub"];
  onReopen: (mode?: "pair") => void;
  statusAvailability?: AppSnapshot["status_availability"];
};

/**
 * Pairing in Ajustes › Cuenta (SIM-24): nothing when the Mac is paired and
 * linked; unpaired or withdrawn says so, beside the way to pair. An unknown
 * link says nothing. Server and device names are not for the screen.
 */
export function AccountPairingControls({ hub, onReopen, statusAvailability }: AccountPairingProps) {
  const es = useSpanish();
  const [confirming, setConfirming] = useState(false);
  const reopenPairing = () => {
    if (hub.paired && hub.connection !== "revoked") {
      setConfirming(true);
      return;
    }
    onReopen("pair");
  };
  const known = statusAvailability?.state !== "unknown" && statusAvailability?.state !== "absent";
  const problem = !known
    ? null
    : hub.connection === "revoked"
      ? es.settings.revoked
      : hub.paired
        ? null
        : es.settings.unpaired;
  return (
    <>
      {problem !== null && <p className={styles.notice}>{problem}</p>}
      {confirming && (
        <div className={styles.warning} role="alert">
          <strong>{es.settings.pairAgainWarning}</strong>
          <div className={styles.actions}>
            <Button onClick={() => onReopen("pair")} variant="accent">
              {es.settings.confirmPairAgain}
            </Button>
            <Button onClick={() => setConfirming(false)} variant="ghost">
              {es.settings.cancelPairAgain}
            </Button>
          </div>
        </div>
      )}
      <div className={styles.actions}>
        <Button onClick={reopenPairing}>{es.settings.pairAgain}</Button>
        <Button onClick={() => onReopen()} variant="ghost">
          {es.settings.reopenOnboarding}
        </Button>
      </div>
    </>
  );
}
