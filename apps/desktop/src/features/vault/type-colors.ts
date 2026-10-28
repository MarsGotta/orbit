import { vaultCopy as copy } from "../../i18n/es/vault";

export const COLORS: Record<string, string> = {
  person: "var(--o-c1)",
  project: "var(--o-c2)",
  meeting: "var(--o-c4)",
  topic: "var(--o-c3)",
  source: "var(--o-ink3)",
  journal: "var(--o-accText)",
  note: "var(--o-ink2)",
};
export function typeLabel(type: string, role: string) {
  return type === role ? (copy.types[role as keyof typeof copy.types] ?? type) : type;
}
