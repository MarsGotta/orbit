import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it } from "vitest";
import { followTheme, toggleTheme, useTheme } from "./theme";

const KEY = "orbit.ui.theme.local";
let stop: () => void = () => undefined;

beforeEach(() => {
  window.localStorage.clear();
  delete document.documentElement.dataset.theme;
});

afterEach(() => stop());

it("follows the system until a theme is fixed: nothing is set on the page", () => {
  stop = followTheme();
  expect(document.documentElement.dataset.theme).toBeUndefined();
  // jsdom has no `prefers-color-scheme`: the system reads as dark.
  expect(renderHook(() => useTheme()).result.current).toBe("dark");
});

it("changing the theme fixes it on the page and remembers it on this Mac", () => {
  stop = followTheme();
  const { result } = renderHook(() => useTheme());
  act(() => toggleTheme());
  expect(document.documentElement.dataset.theme).toBe("light");
  expect(window.localStorage.getItem(KEY)).toBe("light");
  expect(result.current).toBe("light");
  act(() => toggleTheme());
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(result.current).toBe("dark");
});

it("a window that opens later starts with the remembered theme", () => {
  window.localStorage.setItem(KEY, "light");
  stop = followTheme();
  expect(document.documentElement.dataset.theme).toBe("light");
});

it("the floating panel follows a change made in the main window", () => {
  stop = followTheme();
  const { result } = renderHook(() => useTheme());
  // The other window writes the shared storage; this one hears a `storage` event.
  window.localStorage.setItem(KEY, "light");
  act(() => {
    window.dispatchEvent(new StorageEvent("storage", { key: KEY, newValue: "light" }));
  });
  expect(document.documentElement.dataset.theme).toBe("light");
  expect(result.current).toBe("light");
});
