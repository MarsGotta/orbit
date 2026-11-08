import { numberFormat } from "../locale";
import { getLocale } from "../locale";
/** Thousands with a dot, as Spanish writes them: 16.384. */
const number = (value: number) => value.toLocaleString(getLocale());

/** Settings' lists: the assistant's folders and what a detected call does. */
export const settingsLists = {
  // The assistant's folders: choosing one in the macOS panel is the whole
  // permission; nothing here switches it on or off.
  folders: {
    section: "Folders they can read",
    help: "Shared by all your assistants. They can read these files, never change them.",
    count: (count: number, max: number) =>
      `${numberFormat().format(count)} of ${numberFormat().format(max)}`,
    add: "Add folder",
    opening: "Opening…",
    remove: (name: string) => `Remove ${name}`,
    removeLabel: "Remove",
    empty: "No folders yet. Add one for your assistants to read.",
    accessible: "Readable",
    unknown: "Checking…",
    unreadable: "Couldn’t read saved folders.",
    full: (max: number) =>
      `Up to ${numberFormat().format(max)} folders. Remove one to add another.`,
    notReady: "Orbit is still starting",
    readdHint: "Remove it and add it again.",
    missing: "Not found: it was deleted or its disk is disconnected.",
    notAFolder: "It’s no longer a folder.",
    accessDenied: "macOS won’t let Orbit open it.",
    tooLong: (bytes: number, max: number) =>
      `Its path takes ${number(bytes)} bytes; Orbit allows ${number(max)}.`,
    bookmarkTooLarge: (bytes: number, max: number) =>
      `Its bookmark takes ${number(bytes)} bytes; Orbit allows ${number(max)}.`,
    disk: (name: string) => `Disk “${name}”`,
    // The folders of the home folder, as Finder names them in Spanish.
    homeFolders: { Desktop: "Desktop", Documents: "Documents", Downloads: "Downloads" },
  },
  // «Al detectar una llamada», a preference and not a permission.
  callSettings: {
    options: {
      notify: {
        label: "Notify me (recommended)",
        help: "Orbit asks whether you want to record it.",
      },
      record: {
        label: "Record automatically",
        help: "Starts recording automatically and lets you know.",
      },
      nothing: { label: "Do nothing", help: "Start recording whenever you choose." },
    },
  },
} as const;
