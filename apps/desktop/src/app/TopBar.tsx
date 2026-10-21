import type { ReactNode } from "react";
import { es } from "../i18n/es";
import { Button } from "../design/Button";
import { Icon } from "../design/Icon";
import { Kbd } from "../design/Kbd";
import styles from "./TopBar.module.css";

export interface TopBarProps {
  meetEntry?: ReactNode;
  onOpenPalette: () => void;
  onToggleTheme: () => void;
  /** The status pill: empty when all is well (SIM-24). */
  status?: ReactNode;
  viewTitle: string;
}

export function TopBar({
  meetEntry,
  onOpenPalette,
  onToggleTheme,
  status,
  viewTitle,
}: TopBarProps) {
  const mac = navigator.userAgent.includes("Mac");
  return (
    <header className={`${styles.topBar} ${mac ? styles.mac : ""}`}>
      <nav aria-label={es.shell.breadcrumbsLabel} className={styles.breadcrumbs}>
        <strong>{es.appName}</strong>
        <span aria-hidden="true">/</span>
        <span>{viewTitle}</span>
      </nav>
      <div className={styles.actions}>
        <Button onClick={onOpenPalette} size={28} variant="outline">
          <Icon name="search" size={13} />
          {es.shell.askOrbit}
          <Kbd>⌘⇧␣</Kbd>
        </Button>
        <Button
          aria-label={es.shell.changeTheme}
          onClick={onToggleTheme}
          size={28}
          title={es.shell.themeTitle}
          variant="icon"
        >
          <Icon name="theme" size={14} />
        </Button>
        {meetEntry}
        {status}
      </div>
    </header>
  );
}
