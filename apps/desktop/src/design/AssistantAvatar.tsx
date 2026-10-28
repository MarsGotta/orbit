import styles from "./AssistantAvatar.module.css";

/** 18 over each answer, 20 in the conversation list, 26 in a header, 28 in «Tus asistentes»,
 *  36 on cards and in the first run, 40 in the pencil's menu, 56 in its conversation and settings. */
export type AssistantAvatarSize = 18 | 20 | 26 | 28 | 36 | 40 | 56;
/** Violet, blue, green, amber: the calendar's colours, `--o-c1` to `--o-c4`. */
export type AssistantColor = 1 | 2 | 3 | 4;

export interface AssistantAvatarProps {
  name: string;
  color: AssistantColor;
  size: AssistantAvatarSize;
}

/** An assistant's initial in its colour. Always silent: its name is written beside it. */
export function AssistantAvatar({ name, color, size }: AssistantAvatarProps) {
  // The first character as a person sees it, even outside the basic plane.
  const initial = Array.from(name.trim())[0]?.toLocaleUpperCase("es") ?? "";
  return (
    <span
      aria-hidden="true"
      className={`${styles.avatar} ${styles[`c${color}`]} ${styles[`size${size}`]}`}
    >
      {initial}
    </span>
  );
}
