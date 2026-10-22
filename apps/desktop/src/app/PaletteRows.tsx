import { Icon } from "../design/Icon";
import type { PaletteCommand } from "./palette-commands";
import styles from "./Palette.module.css";

/** A command: its icon (or recording's red dot), title, grey subtitle and shortcut. */
export function CommandRow({ command }: { command: PaletteCommand }) {
  return (
    <>
      <span aria-hidden="true" className={styles.icon}>
        {command.icon === "record" ? (
          <span className={styles.recordDot} />
        ) : (
          <Icon name={command.icon} size={18} />
        )}
      </span>
      <span className={styles.text}>
        <span className={styles.title} data-title>
          {command.title}
        </span>
        {command.subtitle && <span className={styles.subtitle}>{command.subtitle}</span>}
      </span>
      {command.shortcut && <kbd className={styles.shortcut}>{command.shortcut}</kbd>}
    </>
  );
}

/** «Preguntar a Deimos: «…»», sent with ↵. */
export function AskRow({ label, text }: { label: string; text: string }) {
  return (
    <>
      <span aria-hidden="true" className={`${styles.icon} ${styles.askIcon}`}>
        <Icon name="ask" size={18} />
      </span>
      <span className={styles.askText}>
        <span className={styles.title}>{label}</span> <span className={styles.quoted}>{text}</span>
      </span>
      <kbd className={styles.shortcut}>↵</kbd>
    </>
  );
}
