import type { ReactNode } from "react";
import styles from "./Icon.module.css";

/**
 * The redesign's icons: a 24 grid, thin strokes and no fill, in the colour of
 * the text around them. 20 in the sidebar, 16 in buttons and rows, 14 in metadata.
 * Each is its stroke width (null when it is filled) and its shapes.
 */
const icons = {
  briefing: [
    1.7,
    <path d="M3 18.5h18M7 18.5a5 5 0 0 1 10 0M12 5v3M5.5 10l1.6 1.2M18.5 10l-1.6 1.2" />,
  ],
  calendar: [
    1.7,
    <>
      <rect x="3.5" y="5" width="17" height="15.5" rx="3" />
      <path d="M3.5 10h17M8 3v4M16 3v4" />
    </>,
  ],
  notes: [
    1.7,
    <>
      <path d="M14 3.5H7a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5z" />
      <path d="M14 3.5v5h5M9 13h6M9 16.5h4" />
    </>,
  ],
  assistant: [
    1.7,
    <path d="M12 3.5c4.7 0 8.5 3.4 8.5 7.6s-3.8 7.6-8.5 7.6c-1 0-2-.1-2.9-.4L5 20l.9-3.6C4.4 15 3.5 13.1 3.5 11.1 3.5 6.9 7.3 3.5 12 3.5z" />,
  ],
  settings: [
    1.7,
    <>
      <path d="M4 7h9M17 7h3M4 17h3M11 17h9" />
      <circle cx="15" cy="7" r="2" />
      <circle cx="9" cy="17" r="2" />
    </>,
  ],
  search: [
    1.8,
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="M20 20l-4.2-4.2" />
    </>,
  ],
  ask: [1.8, <path d="M12 3.5l1.9 5.6 5.6 1.9-5.6 1.9L12 18.5l-1.9-5.6-5.6-1.9 5.6-1.9z" />],
  mic: [
    1.8,
    <>
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21" />
    </>,
  ],
  phone: [
    1.8,
    <path d="M5 4h3.5l1.8 4.4-2.3 1.4a11 11 0 0 0 6.2 6.2l1.4-2.3L20 15.5V19a2 2 0 0 1-2 2A16 16 0 0 1 3 6a2 2 0 0 1 2-2z" />,
  ],
  attach: [
    1.8,
    <path d="M20 11.5l-7.8 7.8a5 5 0 0 1-7.1-7.1l8.5-8.5a3.3 3.3 0 0 1 4.7 4.7l-8.5 8.5a1.7 1.7 0 0 1-2.4-2.4l7.8-7.8" />,
  ],
  send: [2.2, <path d="M12 19V5M6 11l6-6 6 6" />],
  play: [null, <path d="M8 5.5v13l10.5-6.5z" fill="currentColor" />],
  screen: [
    1.7,
    <>
      <rect x="3" y="4.5" width="18" height="12" rx="2" />
      <path d="M8.5 20h7M12 16.5V20" />
    </>,
  ],
  video: [
    1.7,
    <>
      <rect x="3" y="6.5" width="12.5" height="11" rx="2.5" />
      <path d="M15.5 10.5l5-3v9l-5-3" />
    </>,
  ],
  people: [
    1.8,
    <>
      <circle cx="9" cy="8" r="3.5" />
      <path d="M2.5 20a6.5 6.5 0 0 1 13 0M16 4.6a3.5 3.5 0 0 1 0 6.8M18 14a6.5 6.5 0 0 1 3.5 6" />
    </>,
  ],
  clock: [
    1.7,
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 2" />
    </>,
  ],
  privacy: [1.7, <path d="M12 3l7.5 3v5.5c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5V6z" />],
  stopAll: [
    1.8,
    <>
      <circle cx="12" cy="12" r="8.5" />
      <rect x="9" y="9" width="6" height="6" rx="1" />
    </>,
  ],
  warning: [1.8, <path d="M12 4l9 15.5H3zM12 10v4M12 17v.01" />],
  locked: [
    2,
    <>
      <rect x="5" y="10.5" width="14" height="10" rx="2.5" />
      <path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5" />
    </>,
  ],
  edit: [
    1.8,
    <>
      <path d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16v4z" />
      <path d="M13.5 6.5l4 4" />
    </>,
  ],
  refresh: [
    1.8,
    <path d="M20 11a8 8 0 0 0-14.3-4.3L4 9M4 4v5h5M4 13a8 8 0 0 0 14.3 4.3L20 15M20 20v-5h-5" />,
  ],
  check: [2.6, <path d="M5 12.5l4.5 4.5L19 7.5" />],
  plus: [2.2, <path d="M12 5v14M5 12h14" />],
  // Not in the redesign's set; only the call orb still uses it.
  minus: [2.2, <path d="M5 12h14" />],
  close: [2, <path d="M6 6l12 12M18 6L6 18" />],
  previous: [2, <path d="M15 6l-6 6 6 6" />],
  next: [2, <path d="M9 6l6 6-6 6" />],
  chevron: [2, <path d="M6 9l6 6 6-6" />],
  // The annex's icons: an assistant's settings, the actions under an answer and an image's.
  gear: [
    1.8,
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
    </>,
  ],
  copy: [
    1.8,
    <>
      <rect x="8.5" y="8.5" width="11" height="11" rx="2.5" />
      <path d="M15.5 8.5V6.5a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2" />
    </>,
  ],
  listen: [
    1.8,
    <>
      <path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z" />
      <path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11" />
    </>,
  ],
  // «Parar» while an answer is heard: the prototype's square, not in the annex's files.
  stop: [null, <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" />],
  download: [1.8, <path d="M12 4v11M7 10.5l5 5 5-5M5 19.5h14" />],
  expand: [1.8, <path d="M14 4h6v6M10 20H4v-6M20 4l-7 7M4 20l7-7" />],
  conversation: [1.8, <path d="M21 12a8 8 0 0 1-11.6 7.1L4 20.5l1.4-5A8 8 0 1 1 21 12z" />],
  themeLight: [
    1.8,
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2.5v2M12 19.5v2M4.6 4.6l1.4 1.4M18 18l1.4 1.4M2.5 12h2M19.5 12h2M4.6 19.4L6 18M18 6l1.4-1.4" />
    </>,
  ],
  themeDark: [1.8, <path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z" />],
  logo: [
    null,
    <>
      <circle cx="12" cy="12" r="4.2" fill="currentColor" />
      <ellipse
        cx="12"
        cy="12"
        rx="10"
        ry="4.2"
        stroke="currentColor"
        strokeWidth="1.5"
        transform="rotate(-28 12 12)"
      />
    </>,
  ],
} satisfies Record<string, [number | null, ReactNode]>;

export type IconName = keyof typeof icons;
/** Every icon, in the order above (the showcase draws them all). */
export const iconNames = Object.keys(icons) as IconName[];
/** 15 is the annex's, for the icon buttons of 28 and 30 points. */
export type IconSize = 14 | 15 | 16 | 18 | 20;

export interface IconProps {
  name: IconName;
  size?: IconSize;
  /** Only for an icon that stands alone and means something; beside a text it stays silent. */
  title?: string;
}

export function Icon({ name, size = 20, title }: IconProps) {
  const [stroke, body] = icons[name];
  return (
    <svg
      aria-hidden={title ? undefined : true}
      aria-label={title}
      className={styles.icon}
      fill="none"
      height={size}
      role={title ? "img" : undefined}
      stroke={stroke === null ? undefined : "currentColor"}
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={stroke ?? undefined}
      viewBox="0 0 24 24"
      width={size}
    >
      {body}
    </svg>
  );
}
