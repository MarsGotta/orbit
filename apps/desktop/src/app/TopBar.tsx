import { useState } from "react";
import { currentHubConnection } from "../bridge/hub-state";
import { Icon } from "../design/Icon";
import { toggleTheme, useTheme } from "../design/theme";
import { useSpanish } from "../i18n/AssistantCopy";
import type { AppView as AppSnapshot } from "../state/app-view";
import { useShellSlots } from "./shell-slots";
import { settleHub, topBarNotices } from "./status-problem";
import { TopBarNotices, type NoticeActions } from "./TopBarNotices";
import styles from "./TopBar.module.css";

export interface TopBarProps extends NoticeActions {
  onOpenPalette: () => void;
  snapshot: AppSnapshot;
}

/** The hub link's last settled state, kept while it reconnects. */
function useSettledHub(snapshot: AppSnapshot) {
  const connection = currentHubConnection(snapshot);
  const [settled, setSettled] = useState(() => settleHub(null, connection));
  const next = settleHub(settled, connection);
  if (next !== settled) setSettled(next);
  return next;
}

function ThemeToggle() {
  const es = useSpanish();
  const dark = useTheme() === "dark";
  return (
    <button
      aria-label={dark ? es.shell.toLight : es.shell.toDark}
      className={styles.theme}
      onClick={toggleTheme}
      type="button"
    >
      <Icon name={dark ? "themeLight" : "themeDark"} size={16} />
    </button>
  );
}

/**
 * 52 px over the view: the search box that opens the palette, or the
 * view's own trail where it has depth; the notice of the moment; the theme.
 */
export function TopBar({ onOpenPalette, snapshot, send, onPair }: TopBarProps) {
  const es = useSpanish();
  const slots = useShellSlots();
  const notices = topBarNotices(snapshot, useSettledHub(snapshot));
  return (
    <header className={styles.topBar}>
      <div
        className={styles.leading}
        data-claimed={slots?.leadingClaims ? true : undefined}
        ref={slots?.setLeading}
      />
      {!slots?.leadingClaims && (
        <button className={styles.search} onClick={onOpenPalette} type="button">
          <Icon name="search" size={14} />
          <span className={styles.searchText}>{es.shell.search}</span>
          <kbd className={styles.shortcut}>{es.shell.searchShortcut}</kbd>
        </button>
      )}
      {/* A view's own header fills the bar up to the notices: it ends where it wants. */}
      {!slots?.leadingClaims && <span className={styles.spacer} />}
      <TopBarNotices actions={{ send, onPair }} notices={notices} />
      <ThemeToggle />
    </header>
  );
}
