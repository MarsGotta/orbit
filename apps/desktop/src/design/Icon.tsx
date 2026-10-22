import type { ReactNode } from "react";
import styles from "./Icon.module.css";

export type IconName =
  | "briefing"
  | "calendar"
  | "notes"
  | "assistant"
  | "attach"
  | "search"
  | "settings"
  | "phone"
  | "screen"
  | "mic"
  | "send"
  | "play"
  | "plus"
  | "chevron"
  | "check"
  | "close"
  | "minus"
  | "theme";

const paths: Record<IconName, ReactNode> = {
  briefing: <path d="M12 3l1.8 4.9L19 9.7l-4.2 3 .6 5.3-3.4-2.6-3.4 2.6.6-5.3L5 9.7l5.2-1.8z" />,
  calendar: (
    <>
      <rect x="3.5" y="5" width="17" height="15" rx="3" />
      <path d="M8 3v4M16 3v4M3.5 10h17" />
    </>
  ),
  notes: (
    <>
      <path d="M6 3.5h9l4 4V20a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4.5a1 1 0 0 1 1-1z" />
      <path d="M14.5 3.5V8H19M8.5 12.5h7M8.5 16h5" />
    </>
  ),
  assistant: (
    <>
      <path d="M20.5 12a8.5 8.5 0 1 1-4.1-7.3" />
      <path d="M4.5 19.5l1.3-3.4M17 3v4M15 5h4" />
    </>
  ),
  attach: (
    <path d="M20 11.5l-7.8 7.8a5 5 0 0 1-7.1-7.1l8.5-8.5a3.3 3.3 0 0 1 4.7 4.7l-8.5 8.5a1.7 1.7 0 0 1-2.4-2.4l7.8-7.8" />
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="M20 20l-4-4" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.6 1.6 0 0 0 .33 1.76l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.6 1.6 0 0 0-2.72 1.13V21a2 2 0 1 1-4 0v-.12A1.6 1.6 0 0 0 7.8 19.4a1.6 1.6 0 0 0-1.76.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.6 1.6 0 0 0 3 15a1.6 1.6 0 0 0-1.46-1H1.4a2 2 0 1 1 0-4h.14A1.6 1.6 0 0 0 3 9a1.6 1.6 0 0 0-.33-1.76l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.6 1.6 0 0 0 9 3a1.6 1.6 0 0 0 1-1.46V1.4a2 2 0 1 1 4 0v.14A1.6 1.6 0 0 0 15 3a1.6 1.6 0 0 0 1.76-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.6 1.6 0 0 0 21 9h.12a2 2 0 1 1 0 4H21a1.6 1.6 0 0 0-1.6 1z" />
    </>
  ),
  phone: (
    <path d="M6.5 3.5h3l1.5 4-2 1.5a11 11 0 0 0 5 5L15.5 12l4 1.5v3a2 2 0 0 1-2.2 2A15.5 15.5 0 0 1 4.5 5.7 2 2 0 0 1 6.5 3.5z" />
  ),
  screen: (
    <>
      <rect x="3" y="4" width="18" height="13" rx="2" />
      <path d="M9 21h6" />
    </>
  ),
  mic: (
    <>
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    </>
  ),
  send: <path d="M12 19V5M6 11l6-6 6 6" />,
  play: <path d="M7 4l13 8-13 8z" />,
  plus: <path d="M12 5v14M5 12h14" />,
  chevron: <path d="M6 9l6 6 6-6" />,
  check: <path d="M5 13l4 4 10-10" />,
  close: <path d="M6 18L18 6M6 6l12 12" />,
  minus: <path d="M5 12h14" />,
  theme: <path d="M20 13.5A8 8 0 1 1 10.5 4a6.5 6.5 0 0 0 9.5 9.5z" />,
};

export interface IconProps {
  name: IconName;
  size?: number;
  title?: string;
}

export function Icon({ name, size = 20, title }: IconProps) {
  const filled = name === "play";
  return (
    <svg
      aria-hidden={title ? undefined : true}
      aria-label={title}
      className={styles.icon}
      fill={filled ? "currentColor" : "none"}
      height={size}
      role={title ? "img" : undefined}
      stroke={filled ? "none" : "currentColor"}
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={name === "check" ? 3 : 1.8}
      viewBox="0 0 24 24"
      width={size}
    >
      {paths[name]}
    </svg>
  );
}
