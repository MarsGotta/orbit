import type { AppView as AppSnapshot } from "../state/app-view";
import { Avatar } from "../design/Avatar";
import { Icon, type IconName } from "../design/Icon";

import { es } from "../i18n/es";
import type { ViewId } from "../state/ui";
import styles from "./Rail.module.css";

interface RailItem {
  badge?: string;
  icon: IconName;
  id: ViewId;
  label: string;
}

function badge(count: number): string | undefined {
  return count > 0 ? String(count) : undefined;
}

function primaryItems(snapshot: AppSnapshot): RailItem[] {
  const notesPending = snapshot.notes?.list.filter((note) => note.state !== "ready").length ?? 0;
  const assistantPending = snapshot.chat.pending ? 1 : 0;
  return [
    {
      icon: "briefing",
      id: "briefing",
      label: es.shell.briefingToday,
    },
    { icon: "calendar", id: "calendar", label: es.shell.calendar },
    { badge: badge(notesPending), icon: "notes", id: "notes", label: es.shell.notes },
    {
      badge: badge(assistantPending),
      icon: "assistant",
      id: "assistant",
      label: es.shell.assistant,
    },
  ];
}

function RailButton({
  active,
  item,
  onNavigate,
}: {
  active: boolean;
  item: RailItem;
  onNavigate: (view: ViewId) => void;
}) {
  return (
    <button
      aria-current={active ? "page" : undefined}
      aria-label={item.label}
      className={styles.item}
      onClick={() => onNavigate(item.id)}
      title={item.label}
      type="button"
    >
      <Icon name={item.icon} />
      <span className={styles.visuallyHidden}>{item.label}</span>
      {item.badge && <span className={styles.badge}>{item.badge}</span>}
    </button>
  );
}

export function Rail({
  activeView,
  onNavigate,
  snapshot,
}: {
  activeView: ViewId;
  onNavigate: (view: ViewId) => void;
  snapshot: AppSnapshot;
}) {
  const settings = { icon: "settings", id: "settings", label: es.shell.settings } as const;
  const userName = snapshot.settings.profile?.name?.trim() || es.shell.localUser;
  const userInitial = userName.charAt(0).toUpperCase();
  return (
    <nav aria-label={es.shell.viewsLabel} className={styles.rail}>
      {primaryItems(snapshot).map((item) => (
        <RailButton
          active={activeView === item.id}
          item={item}
          key={item.id}
          onNavigate={onNavigate}
        />
      ))}
      <span className={styles.spacer} />
      <RailButton active={activeView === settings.id} item={settings} onNavigate={onNavigate} />
      <Avatar initial={userInitial} label={userName} size={30} />
    </nav>
  );
}
