import { schemaProblemText } from "./schemaRefusalDetails";
import { formatQuantity } from "../locale";
import type { VaultDiagnostic } from "../../bridge/generated/VaultDiagnostic";

export function vaultWarningText(diagnostic: VaultDiagnostic): string {
  const { code, field, numbers } = diagnostic;

  if (code === "query_line_unsupported") return `This query line is not applied: ${field}`;
  if (code === "query_page_limit")
    return `The query shows ${formatQuantity(numbers.max)} of ${formatQuantity(numbers.count)} tasks; narrow the filters to see the rest.`;
  const cause = schemaProblemText(diagnostic);
  if (cause === undefined) return `Orbit does not recognise this warning code: ${code}.`;
  return `SCHEMA.md: ${cause}. Defaults are used.`;
}
