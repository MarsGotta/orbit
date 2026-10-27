import { useEffect, useState } from "react";

export interface VaultRoute {
  section: "home" | "tasks" | "map" | "note";
  path: string;
  markdown: boolean;
}
export function readRoute(): VaultRoute {
  const route = window.location.hash.slice(1);
  const [path = "", query = ""] = route.split("?");
  const section = path.startsWith("/vault/n/")
    ? "note"
    : path === "/vault/tasks"
      ? "tasks"
      : path === "/vault/map"
        ? "map"
        : "home";
  let note = "";
  try {
    if (section === "note") note = decodeURIComponent(path.slice("/vault/n/".length));
  } catch {
    note = "";
  }
  return { section, path: note, markdown: new URLSearchParams(query).get("md") === "1" };
}
export function noteRoute(path: string, markdown = false): string {
  return `#/vault/n/${encodeURIComponent(path.replace(/^\//, ""))}${markdown ? "?md=1" : ""}`;
}
export function openNote(path: string) {
  window.location.hash = noteRoute(path);
}
export function useVaultRoute() {
  const [route, setRoute] = useState(readRoute);
  useEffect(() => {
    const changed = () => setRoute(readRoute());
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  return route;
}
