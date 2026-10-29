import { useSyncExternalStore } from "react";

export type Theme = "dark" | "light";
export type ThemeChoice = Theme | "system";

/** Shared by both windows of the app: the floating panel reads what the main window wrote. */
const THEME_KEY = "orbit.ui.theme.local";
const SYSTEM_LIGHT = "(prefers-color-scheme: light)";

// Undefined reads storage; null follows the system when storage refused the choice.
let fixedHere: Theme | null | undefined;
const listeners = new Set<() => void>();

function fixedTheme(): Theme | null {
  if (fixedHere !== undefined) return fixedHere;
  try {
    const stored = window.localStorage.getItem(THEME_KEY);
    return stored === "light" || stored === "dark" ? stored : null;
  } catch {
    return null;
  }
}

function systemTheme(): Theme {
  try {
    return window.matchMedia?.(SYSTEM_LIGHT).matches ? "light" : "dark";
  } catch {
    return "dark";
  }
}

/** The theme on screen: the one fixed on this Mac, or the system's. */
export function currentTheme(): Theme {
  return fixedTheme() ?? systemTheme();
}

function themeChoice(): ThemeChoice {
  return fixedTheme() ?? "system";
}

/** Without a fixed theme nothing is set and the stylesheet follows the system. */
function apply(): void {
  const fixed = fixedTheme();
  if (fixed) document.documentElement.dataset.theme = fixed;
  else delete document.documentElement.dataset.theme;
  for (const listener of listeners) listener();
}

/** Fixes the other theme, from the top bar or from Settings, and remembers it. */
export function toggleTheme(): void {
  chooseTheme(currentTheme() === "dark" ? "light" : "dark");
}

/** System removes the fixed preference so both windows follow macOS again. */
export function chooseTheme(next: ThemeChoice): void {
  fixedHere = next === "system" ? null : next;
  try {
    if (next === "system") window.localStorage.removeItem(THEME_KEY);
    else window.localStorage.setItem(THEME_KEY, next);
    fixedHere = undefined;
  } catch {
    // The choice stays in effect for this window while storage is unavailable.
  }
  apply();
}

/**
 * Puts the remembered theme on this window and keeps it in step with the
 * other window and with the system. Returns what stops following.
 */
export function followTheme(): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === THEME_KEY || event.key === null) apply();
  };
  const media = typeof window.matchMedia === "function" ? window.matchMedia(SYSTEM_LIGHT) : null;
  window.addEventListener("storage", onStorage);
  media?.addEventListener("change", apply);
  apply();
  return () => {
    window.removeEventListener("storage", onStorage);
    media?.removeEventListener("change", apply);
  };
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useTheme(): Theme {
  return useSyncExternalStore(subscribe, currentTheme);
}

export function useThemeChoice(): ThemeChoice {
  return useSyncExternalStore(subscribe, themeChoice);
}
