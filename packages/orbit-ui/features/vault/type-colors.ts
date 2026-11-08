import { getCopy } from "../../i18n";

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
  const copy = getCopy().extras.vaultCopy;
  return type === role ? (copy.types[role as keyof typeof copy.types] ?? type) : type;
}
