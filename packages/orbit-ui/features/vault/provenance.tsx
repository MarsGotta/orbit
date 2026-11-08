import { createContext, useContext } from "react";
import { getCopy } from "../../i18n";
import { getLocale } from "../../i18n/locale";
export const VaultOwner = createContext("");
export function normalizeOwner(value: string) {
  return value
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}
export function actorName(by: string, owner: string) {
  const copy = getCopy().distillation;
  if (by.startsWith("orbit-hub/") || by.startsWith("orbit/") || by.startsWith("deimos/"))
    return copy.author;
  if (by.startsWith("human:"))
    return owner !== "" && by.slice(6) === owner
      ? copy.you
      : by.slice(6).replaceAll("-", " ") || getCopy().vaultReading.noData;
  return by.split("/")[0] ?? by;
}
export function provenanceDate(value: string) {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return value;
  const now = new Date();
  const day =
    date.toDateString() === now.toDateString()
      ? getCopy().distillation.today
      : date.toLocaleDateString(getLocale(), { day: "numeric", month: "short" });
  return `${day}, ${date.toLocaleTimeString(getLocale(), { hour: "2-digit", minute: "2-digit" })}`;
}
export function useVaultOwner() {
  return useContext(VaultOwner);
}
export function actor(value: unknown, owner: string, verified = false): string {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return getCopy().vaultReading.noData;
  const event = value as { by?: unknown; at?: unknown };
  const by = typeof event.by === "string" ? event.by : "";
  const label =
    verified && owner !== "" && by === `human:${owner}`
      ? getCopy().distillation.byYou
      : actorName(by, owner);
  return `${label}${typeof event.at === "string" ? ` · ${provenanceDate(event.at)}` : ""}`;
}
