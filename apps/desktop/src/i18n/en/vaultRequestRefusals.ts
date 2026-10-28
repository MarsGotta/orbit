import { formatQuantity } from "../locale";

export function vaultRequestRefusal(
  numbers: Record<string, number>,
  fields: Record<string, string | undefined>,
): string {
  const rules: Record<string, string> = {
    missing: "is missing a required value",
    extra_forbidden: "includes an unknown field",
    string_too_long: "exceeds the allowed length",
    literal_error: "is not an allowed option",
    greater_than_equal: "is below the minimum",
    less_than_equal: "exceeds the maximum",
  };
  const rule = fields.rule;
  if (numbers.max_page_size === undefined)
    return "Check the Notes fields: the request has an unsupported format.";
  if (rule === undefined || fields.field === undefined)
    return `Check the Notes fields: 1 to ${formatQuantity(numbers.max_page_size)} entries per page and a vault-relative path.`;
  const reason = Object.hasOwn(rules, rule) ? rules[rule] : "has an unsupported format";
  return `Field “${fields.field}” ${reason}. Pages allow 1 to ${formatQuantity(numbers.max_page_size)} entries.`;
}
