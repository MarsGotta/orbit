import { formatQuantity } from "../locale";
import type { VaultDiagnostic } from "../../bridge/generated/VaultDiagnostic";

export function schemaProblemText(diagnostic: VaultDiagnostic): string | undefined {
  const { code, field, numbers, value = "" } = diagnostic;
  const causes: Record<string, string> = {
    schema_frontmatter_unclosed: `frontmatter closing delimiter missing`,
    schema_yaml_unsafe: `frontmatter contains YAML aliases, anchors or tags`,
    schema_yaml_invalid: `invalid YAML`,
    schema_yaml_depth: `frontmatter exceeds supported YAML depth`,
    schema_frontmatter_mapping: `frontmatter must be a field mapping`,
    schema_yaml_key_type: `non-scalar YAML key`,
    schema_yaml_key_duplicate: `duplicate YAML key`,
    schema_types_mapping: `types must be a mapping`,
    schema_type_name: `types contains an empty name`,
    schema_type_mapping: `${field} must be a mapping with role`,
    schema_type_role: `${field} must be one of person, project, meeting, topic, source, journal, note`,
    schema_type_fields: `${field} contains fields other than role and folder`,
    schema_type_folder: `${field} must be a relative folder ending in /`,
    schema_folders_duplicate: `types contains duplicate folders`,
    schema_fields_mapping: `fields must be a mapping`,
    schema_field_unknown: `${field} is not an Orbit field`,
    schema_field_reserved: `${field} cannot use the OKF field ${value}`,
    schema_field_name: `${field} must be a field name`,
  };
  if (!Object.hasOwn(causes, code)) return undefined;
  const inline = [
    "schema_yaml_invalid",
    "schema_yaml_key_type",
    "schema_yaml_key_duplicate",
  ].includes(code);
  const line =
    numbers.line === undefined
      ? ""
      : inline
        ? ` at line ${formatQuantity(numbers.line)}`
        : ` (line ${formatQuantity(numbers.line)})`;
  return `${causes[code]}${line}`;
}
