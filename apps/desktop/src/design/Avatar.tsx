import styles from "./Avatar.module.css";

export type AvatarSize = 18 | 30;
export type AvatarColor = "accent" | "orange" | "blue" | "green" | "gray";

export interface AvatarProps {
  color?: AvatarColor;
  initial: string;
  label: string;
  size?: AvatarSize;
}

export function Avatar({ color = "accent", initial, label, size = 30 }: AvatarProps) {
  return (
    <span
      aria-label={label}
      className={`${styles.avatar} ${styles[color]} ${styles[`size${size}`]}`}
      role="img"
    >
      {initial.slice(0, 1).toUpperCase()}
    </span>
  );
}
