import { Icon, type IconName } from "./Icon";
import styles from "./SidebarItem.module.css";

export interface SidebarItemProps {
  href: string;
  icon: IconName;
  label: string;
  active: boolean;
  /** A short fact at the right: «1 en curso». */
  meta?: string;
  /** Or something that needs the person: «Te necesita» (`rec`), «Falta 1» (`warn`). */
  alert?: { tone: "rec" | "warn"; text: string };
}

export function SidebarItem({ href, icon, label, active, meta, alert }: SidebarItemProps) {
  return (
    <a aria-current={active ? "page" : undefined} className={styles.item} href={href}>
      <span className={styles.icon}>
        <Icon name={icon} size={18} />
      </span>
      <span className={styles.label}>{label}</span>
      {alert ? (
        <span className={styles.alert} data-tone={alert.tone}>
          <span aria-hidden="true">● </span>
          {alert.text}
        </span>
      ) : (
        meta && <span className={styles.meta}>{meta}</span>
      )}
    </a>
  );
}
