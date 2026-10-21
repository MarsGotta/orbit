import type { AppCommand, SettingsSnapshot } from "../../bridge/generated";
import { MicrophoneRow } from "./rows/MicrophoneRow";
import styles from "./SettingsView.module.css";

interface Props {
  daemonReady: boolean;
  send: (command: AppCommand) => void;
  settings: SettingsSnapshot;
}

// REF-11: no row switches the screen, the control or the meetings on. The
// macOS permissions decide, and «Permisos del sistema» shows and opens them.
export function AutonomySettings({ daemonReady, send, settings }: Props) {
  return (
    <section className={styles.controls}>
      <MicrophoneRow daemonReady={daemonReady} send={send} settings={settings} />
    </section>
  );
}
