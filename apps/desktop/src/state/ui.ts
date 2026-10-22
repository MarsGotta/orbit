import { create } from "zustand";

export type ViewId = "briefing" | "calendar" | "notes" | "assistant" | "settings";

/** The sections of Ajustes, each with its own route: `#/settings/{section}`. */
export type SettingsSection = "general" | "privacidad" | "grabacion" | "asistente" | "reuniones";

export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  "general",
  "privacidad",
  "grabacion",
  "asistente",
  "reuniones",
];

const VIEWS: readonly ViewId[] = ["briefing", "calendar", "notes", "assistant", "settings"];

/** The route of a view; Ajustes without a section opens General. */
export function routeFor(view: ViewId): string {
  return `#/${view}`;
}

/** The route of one section of Ajustes, where a notice that it fixes leads. */
export function settingsRoute(section: SettingsSection): string {
  return `#/settings/${section}`;
}

interface Route {
  view: ViewId;
  section: SettingsSection;
}

/**
 * The view and the section a route names. Its first part is the view
 * (`#/notes/<id>` is Reuniones with a note open); an unknown view is Hoy and
 * an unknown section of Ajustes is General.
 */
export function parseRoute(hash: string): Route {
  const [first = "", second = ""] = hash.replace(/^#\/?/, "").split("/");
  const view = VIEWS.includes(first as ViewId) ? (first as ViewId) : "briefing";
  const section =
    view === "settings" && SETTINGS_SECTIONS.includes(second as SettingsSection)
      ? (second as SettingsSection)
      : "general";
  return { view, section };
}

interface UiState {
  view: ViewId;
  settingsSection: SettingsSection;
  selectedNote: { recordingId: string; title: string } | null;
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

function routeState(): Pick<UiState, "view" | "settingsSection"> {
  const { view, section } = parseRoute(window.location.hash);
  return { view, settingsSection: section };
}

export function initializeUiStore(): () => void {
  useUiStore.setState({ ...routeState(), selectedCalendarEventId: null });
  const onHashChange = () => useUiStore.setState(routeState());
  window.addEventListener("hashchange", onHashChange);
  return () => window.removeEventListener("hashchange", onHashChange);
}
