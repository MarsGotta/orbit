import { create } from "zustand";

const SIDEBAR_KEY = "orbit.ui.sidebar.local";
const SCALE_KEY = "orbit.ui.reading-scale.local";

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function remember(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* Keep the choice in this window. */
  }
}

interface Preferences {
  sidebar: boolean;
  scale: number;
  toggleSidebar: () => void;
  resizeText: (step: number | "reset") => void;
}

/** Only reading text grows; controls, navigation and window geometry keep their size. */
export const useWindowPreferences = create<Preferences>((set, get) => ({
  sidebar: true,
  scale: 1,
  toggleSidebar: () => {
    const sidebar = !get().sidebar;
    remember(SIDEBAR_KEY, String(sidebar));
    set({ sidebar });
  },
  resizeText: (step) => {
    // 100–200% in ten-percent steps: preserve the designed minimum and expose 200% reading.
    const scale =
      step === "reset" ? 1 : Math.max(1, Math.min(2, Math.round((get().scale + step) * 10) / 10));
    remember(SCALE_KEY, String(scale));
    set({ scale });
  },
}));

/** Local presentation preferences, shared with the floating window through storage. */
export function followWindowPreferences(): () => void {
  const load = () => {
    const scale = Number(read(SCALE_KEY) ?? 1);
    useWindowPreferences.setState({
      sidebar: read(SIDEBAR_KEY) !== "false",
      scale: Number.isFinite(scale) && scale >= 1 && scale <= 2 ? scale : 1,
    });
  };
  const apply = () =>
    document.documentElement.style.setProperty(
      "--o-reading-scale",
      String(useWindowPreferences.getState().scale),
    );
  load();
  apply();
  const stop = useWindowPreferences.subscribe(apply);
  const storage = (event: StorageEvent) => {
    if (event.key === null || event.key === SCALE_KEY || event.key === SIDEBAR_KEY) load();
  };
  window.addEventListener("storage", storage);
  return () => {
    stop();
    window.removeEventListener("storage", storage);
  };
}
