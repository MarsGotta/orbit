import { create } from "zustand";

export type Theme = "dark" | "light";
export type ViewId = "briefing" | "calendar" | "notes" | "assistant" | "settings";

interface UiState {
  theme: Theme;
  view: ViewId;
  selectedNote: { recordingId: string; title: string } | null;
  selectedCalendarEventId: string | null;
  navigate: (view: ViewId) => void;
  openNote: (recordingId: string, title: string) => void;
  openCalendarEvent: (eventId: string) => void;
  clearCalendarEvent: () => void;
  toggleTheme: () => void;
}

const themeKey = "orbit.ui.theme.local";

function viewFromHash(hash: string): ViewId {
  const route = hash.replace(/^#\/?/, "");
  const views: ViewId[] = ["briefing", "calendar", "notes", "assistant", "settings"];
  return views.includes(route as ViewId) ? (route as ViewId) : "briefing";
}

function hashForView(view: ViewId): string {
  return `#/${view}`;
}

function systemTheme(): Theme {
  try {
    return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
  } catch {
    return "dark";
  }
}

function storedTheme(): Theme {
  try {
    const stored = window.localStorage.getItem(themeKey);
    return stored === "light" || stored === "dark" ? stored : systemTheme();
  } catch {
    return systemTheme();
  }
}

function persistTheme(theme: Theme): void {
  try {
    window.localStorage.setItem(themeKey, theme);
  } catch {
    // The preference stays in effect for this session when storage is unavailable.
  }
}

export const useUiStore = create<UiState>((set, get) => ({
  theme: "dark",
  view: "briefing",
  selectedNote: null,
  selectedCalendarEventId: null,
  navigate: (view) => {
    set({ view, selectedCalendarEventId: null });
    const nextHash = hashForView(view);
    if (window.location.hash !== nextHash) window.location.hash = nextHash;
  },
  openNote: (recordingId, title) => {
    set({ selectedNote: { recordingId, title }, view: "notes" });
    if (window.location.hash !== "#/notes") window.location.hash = "#/notes";
  },
  openCalendarEvent: (eventId) => {
    set({ selectedCalendarEventId: eventId, view: "calendar" });
    if (window.location.hash !== "#/calendar") window.location.hash = "#/calendar";
  },
  clearCalendarEvent: () => set({ selectedCalendarEventId: null }),
  toggleTheme: () => {
    const theme = get().theme === "dark" ? "light" : "dark";
    persistTheme(theme);
    set({ theme });
  },
}));

export function initializeUiStore(): () => void {
  useUiStore.setState({
    theme: storedTheme(),
    view: viewFromHash(window.location.hash),
    selectedCalendarEventId: null,
  });
  const onHashChange = () => useUiStore.setState({ view: viewFromHash(window.location.hash) });
  window.addEventListener("hashchange", onHashChange);
  return () => window.removeEventListener("hashchange", onHashChange);
}
