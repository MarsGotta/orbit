import { formatQuantity } from "../locale";

export function vaultRequestRefusal(
  numbers: Record<string, number>,
  fields: Record<string, string | undefined>,
): string {
  const rules: Record<string, string> = {
    missing: "falta un valor obligatorio",
    extra_forbidden: "incluye un campo desconocido",
    string_too_long: "supera la longitud admitida",
    literal_error: "no es una opción admitida",
    greater_than_equal: "es menor que el mínimo",
    less_than_equal: "supera el máximo",
  };
  const rule = fields.rule;
  if (numbers.max_page_size === undefined)
    return "Revisa los campos de Notas: la petición no tiene el formato admitido.";
  if (rule === undefined || fields.field === undefined)
    return `Revisa los campos de Notas: página de 1 a ${formatQuantity(numbers.max_page_size)} entradas y ruta relativa al vault.`;
  const reason = Object.hasOwn(rules, rule) ? rules[rule] : "no tiene el formato admitido";
  return `El campo «${fields.field}» ${reason}. Las páginas admiten de 1 a ${formatQuantity(numbers.max_page_size)} entradas.`;
}
