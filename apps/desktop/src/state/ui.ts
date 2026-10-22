import { create } from "zustand";

export type ViewId = "briefing" | "calendar" | "notes" | "assistant" | "settings";

interface UiState {
  view: ViewId;
  selectedNote: { recordingId: string; title: string } | null;
  selectedCalendarEventId: string | null;
  navigate: (view: ViewId) => void;
  openNote: (recordingId: string, title: string) => void;
  openCalendarEvent: (eventId: string) => void;
  clearCalendarEvent: () => void;
}

function viewFromHash(hash: string): ViewId {
  const route = hash.replace(/^#\/?/, "");
  const views: ViewId[] = ["briefing", "calendar", "notes", "assistant", "settings"];
  return views.includes(route as ViewId) ? (route as ViewId) : "briefing";
}

function hashForView(view: ViewId): string {
  return `#/${view}`;
}

export const useUiStore = create<UiState>((set) => ({
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
}));

export function initializeUiStore(): () => void {
  useUiStore.setState({
    view: viewFromHash(window.location.hash),
    selectedCalendarEventId: null,
  });
  const onHashChange = () => useUiStore.setState({ view: viewFromHash(window.location.hash) });
  window.addEventListener("hashchange", onHashChange);
  return () => window.removeEventListener("hashchange", onHashChange);
}
