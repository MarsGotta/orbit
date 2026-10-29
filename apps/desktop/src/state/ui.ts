import { create } from "zustand";
import type { NoteTab } from "../features/notes/note-tabs";

export type ViewId = "briefing" | "calendar" | "notes" | "vault" | "assistant" | "settings";

/** The sections of Settings, each with its own route: `#/settings/{section}`. */
export type SettingsSection =
  "general" | "privacidad" | "grabacion" | "asistentes" | "reuniones" | "notas" | "ia";

export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  "general",
  "privacidad",
  "grabacion",
  "asistentes",
  "ia",
  "reuniones",
  "notas",
];

/** Catalogue keys use English; existing section URLs remain stable. */
export function settingsSectionKey(section: SettingsSection) {
  const keys = {
    general: "general",
    privacidad: "privacy",
    grabacion: "recording",
    asistentes: "assistants",
    reuniones: "meetings",
    notas: "notes",
    ia: "ai",
  } as const;
  return keys[section];
}

/** Routes of sections that were renamed, kept so an old link still opens its section. */
const RENAMED_SECTIONS: Readonly<Record<string, SettingsSection>> = { asistente: "asistentes" };

const VIEWS: readonly ViewId[] = [
  "briefing",
  "calendar",
  "notes",
  "vault",
  "assistant",
  "settings",
];

/** The route of a view; Settings without a section opens General. */
export function routeFor(view: ViewId): string {
  return `#/${view}`;
}

/** The route of one section of Settings, where a notice that it fixes leads. */
export function settingsRoute(section: SettingsSection): string {
  return `#/settings/${section}`;
}

interface Route {
  view: ViewId;
  section: SettingsSection;
}

/**
 * The view and the section a route names. Its first part is the view
 * (`#/notes/<id>` is Meetings with a note open); an unknown view is Today and
 * an unknown section of Settings is General.
 */
export function parseRoute(hash: string): Route {
  const [first = "", second = ""] = hash.replace(/^#\/?/, "").split("/");
  const view = VIEWS.includes(first as ViewId) ? (first as ViewId) : "briefing";
  const named = RENAMED_SECTIONS[second] ?? second;
  const section =
    view === "settings" && SETTINGS_SECTIONS.includes(named as SettingsSection)
      ? (named as SettingsSection)
      : "general";
  return { view, section };
}

interface UiState {
  view: ViewId;
  settingsSection: SettingsSection;
  selectedNote: { recordingId: string; title: string } | null;
  noteTabs: Record<string, NoteTab>;
  selectNoteTab: (recordingId: string, tab: NoteTab) => void;
  selectedCalendarEventId: string | null;
  navigate: (view: ViewId) => void;
  openSettings: (section: SettingsSection) => void;
  openNote: (recordingId: string, title: string) => void;
  openCalendarEvent: (eventId: string) => void;
  clearCalendarEvent: () => void;
}

function go(hash: string): void {
  if (window.location.hash !== hash) window.location.hash = hash;
}

export const useUiStore = create<UiState>((set) => ({
  view: "briefing",
  settingsSection: "general",
  selectedNote: null,
  noteTabs: {},
  selectNoteTab: (recordingId, tab) =>
    set((state) => ({ noteTabs: { ...state.noteTabs, [recordingId]: tab } })),
  selectedCalendarEventId: null,
  navigate: (view) => {
    set({ view, settingsSection: "general", selectedCalendarEventId: null });
    go(routeFor(view));
  },
  openSettings: (section) => {
    set({ view: "settings", settingsSection: section, selectedCalendarEventId: null });
    go(settingsRoute(section));
  },
  openNote: (recordingId, title) => {
    set({ selectedNote: { recordingId, title }, view: "notes" });
    go("#/notes");
  },
  openCalendarEvent: (eventId) => {
    set({ selectedCalendarEventId: eventId, view: "calendar" });
    go("#/calendar");
  },
  clearCalendarEvent: () => set({ selectedCalendarEventId: null }),
}));

/** The route's state; an old section's route is corrected to its new one in place. */
function routeState(): Pick<UiState, "view" | "settingsSection"> {
  const { view, section } = parseRoute(window.location.hash);
  const [, second = ""] = window.location.hash.replace(/^#\/?/, "").split("/");
  if (view === "settings" && RENAMED_SECTIONS[second] !== undefined)
    window.history.replaceState(null, "", settingsRoute(section));
  return { view, settingsSection: section };
}

export function initializeUiStore(): () => void {
  useUiStore.setState({ ...routeState(), selectedCalendarEventId: null });
  const onHashChange = () => useUiStore.setState(routeState());
  window.addEventListener("hashchange", onHashChange);
  return () => window.removeEventListener("hashchange", onHashChange);
}
