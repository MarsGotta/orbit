import { schemaProblemText } from "./schemaRefusalDetails";
import { formatQuantity } from "../locale";
import type { VaultDiagnostic } from "../../bridge/generated/VaultDiagnostic";

export function vaultWarningText(diagnostic: VaultDiagnostic): string {
  const { code, field, numbers } = diagnostic;

  if (code === "query_line_unsupported") return `Esta línea de la consulta no se aplica: ${field}`;
  if (code === "query_page_limit")
    return `La consulta muestra ${formatQuantity(numbers.max)} de ${formatQuantity(numbers.count)} tareas; acota los filtros para ver las demás.`;
  const cause = schemaProblemText(diagnostic);
  if (cause === undefined) return `Orbit no reconoce este código de aviso: ${code}.`;
  return `SCHEMA.md: ${cause}. Se usan los valores por defecto.`;
}
