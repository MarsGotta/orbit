import type { ReactNode } from "react";
import type { AppCommand, SettingsSnapshot } from "../../../bridge/generated";
import styles from "../SettingsView.module.css";

export interface RowProps {
  action: ReactNode;
  description: string;
  note?: string;
  title: string;
}

export interface ControlProps {
  daemonReady: boolean;
  send: (command: AppCommand) => void;
  settings: SettingsSnapshot;
}

export function SettingsRow({ action, description, note, title }: RowProps) {
  return (
    <div className={styles.controlRow}>
      <div className={styles.rowCopy}>
        <strong>{title}</strong>
        <span>{description}</span>
        {Boolean(note) && <small>{note}</small>}
      </div>
      {action}
    </div>
  );
}
