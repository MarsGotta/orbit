import { formatQuantity } from "../locale";
import type { VaultDiagnostic } from "../../bridge/generated/VaultDiagnostic";

export function schemaProblemText(diagnostic: VaultDiagnostic): string | undefined {
  const { code, field, numbers, value = "" } = diagnostic;
  const causes: Record<string, string> = {
    schema_frontmatter_unclosed: `falta el cierre del frontmatter`,
    schema_yaml_unsafe: `el frontmatter contiene alias, anclas o etiquetas YAML`,
    schema_yaml_invalid: `YAML inválido`,
    schema_yaml_depth: `el frontmatter supera la profundidad YAML admitida`,
    schema_frontmatter_mapping: `el frontmatter debe ser un mapa de campos`,
    schema_yaml_key_type: `clave YAML no escalar`,
    schema_yaml_key_duplicate: `clave YAML duplicada`,
    schema_types_mapping: `types debe ser un mapa`,
    schema_type_name: `types contiene un nombre vacío`,
    schema_type_mapping: `${field} debe ser un mapa con role`,
    schema_type_role: `${field} debe ser uno de person, project, meeting, topic, source, journal, note`,
    schema_type_fields: `${field} contiene campos distintos de role y folder`,
    schema_type_folder: `${field} debe ser una carpeta relativa terminada en /`,
    schema_folders_duplicate: `types contiene carpetas duplicadas`,
    schema_fields_mapping: `fields debe ser un mapa`,
    schema_field_unknown: `${field} no es un campo de Orbit`,
    schema_field_reserved: `${field} no puede usar el campo de OKF ${value}`,
    schema_field_name: `${field} debe ser un nombre de campo`,
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
        ? ` en la línea ${formatQuantity(numbers.line)}`
        : ` (línea ${formatQuantity(numbers.line)})`;
  return `${causes[code]}${line}`;
}
