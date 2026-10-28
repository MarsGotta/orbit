import { schemaProblemText } from "./schemaRefusalDetails";
import { vaultRequestRefusal } from "./vaultRequestRefusals";
import { formatQuantity } from "../locale";

const messages = {
  vault_folder_absolute: "Indica una ruta absoluta a una carpeta del hub.",
  vault_folder_missing: "La carpeta del vault no existe en el hub.",
  vault_folder_permission: "El hub no tiene permiso para leer la carpeta del vault.",
  vault_path_refused:
    "La ruta debe estar dentro del vault, sin archivos ocultos ni enlaces simbólicos.",
  vault_note_missing: "La nota no existe en el vault.",
  vault_not_markdown: "Solo se pueden leer notas Markdown (.md).",
  vault_encoding: "La nota no está codificada en UTF-8.",
  vault_folder_changed: "Una carpeta cambió o no se puede leer; vuelve a buscar.",
  vault_sync_connection: "No se pudo conectar con Fast Note Sync; revisa la URL y el servicio.",
  vault_sync_shape: "Fast Note Sync devolvió una respuesta con formato no válido.",
  vault_sync_missing: "Fast Note Sync no encuentra ese vault.",
  vault_token_refused: "Fast Note Sync rechazó el token; revisa su vigencia y acceso al vault.",
  vault_token_expired: "El token de Fast Note Sync ha caducado.",
  vault_sync_refused: "Fast Note Sync rechazó la lectura del vault.",
  vault_not_connected: "Conecta una carpeta del hub o Fast Note Sync en Notas.",
  vault_frontmatter_invalid: "El frontmatter YAML de la nota no es válido.",
  vault_path_controls: "La ruta del vault contiene caracteres de control.",
  vault_path_components: "La carpeta no admite componentes «.» ni «..».",
  vault_path_symlink: "La carpeta o nota no se puede abrir sin seguir enlaces simbólicos.",
  vault_path_regular_file: "La nota debe ser un archivo normal del vault.",
  vault_sync_note_missing: "La nota no existe en Fast Note Sync.",
  vault_sync_token_required: "Fast Note Sync requiere un token válido.",
  vault_sync_list_shape: "La lista de Fast Note Sync no tiene el formato esperado.",
  vault_sync_note_shape: "Fast Note Sync devolvió una nota con formato o ruta distintos.",
  vault_sync_links_shape: "Fast Note Sync devolvió enlaces con formato no válido.",
  vault_sync_link_path: "Fast Note Sync devolvió un enlace sin ruta.",
} satisfies Record<string, string>;

export function vaultRefusalText(
  code: string,
  numbers: Record<string, number>,
  fields: Record<string, string | undefined> = {},
): string {
  const schema = schemaProblemText({
    code,
    field: fields.field ?? "",
    numbers,
    value: fields.value,
  });
  if (schema !== undefined) return `El frontmatter YAML de la nota: ${schema}.`;
  const n = (key: string) => formatQuantity(numbers[key]);
  if (code === "vault_invalid_request") return vaultRequestRefusal(numbers, fields);
  switch (code) {
    case "vault_sync_catalog_incomplete":
      return `El catálogo de Fast Note Sync no termina dentro del límite.`;
    case "vault_sync_no_response":
      return `Fast Note Sync no respondió dentro de ${n("timeout_seconds")} segundos.`;
    case "vault_sync_response_too_large":
      return `La respuesta de Fast Note Sync supera ${n("max_bytes")} bytes.`;
    case "vault_sync_scan_limit":
      return `El vault supera ${n("max_entries")} entradas por recorrido.`;
    case "vault_sync_links_limit":
      return `La nota supera ${n("max_entries")} enlaces.`;
    case "vault_index_scan_bytes":
      return `El índice supera ${n("max_bytes")} bytes leídos por recorrido.`;

    case "vault_note_too_large":
      return `La nota ocupa ${n("bytes")} bytes; el máximo es ${n("max_bytes")}.`;
    case "vault_scan_limit":
      return `El recorrido supera ${n("max_entries")} entradas.`;
    case "vault_depth_limit":
      return `El vault supera ${n("max_depth")} niveles de carpetas.`;
    case "vault_scan_bytes":
      return `La búsqueda supera ${n("max_bytes")} bytes leídos; acota la carpeta.`;
    case "vault_sync_timeout":
      return `La lectura de Fast Note Sync superó ${n("timeout_seconds")} segundos.`;
    case "vault_sync_http":
      return `Fast Note Sync respondió HTTP ${n("upstream_status")}.`;
    case "vault_response_too_large":
      return `La respuesta de Notas ocupa ${n("bytes")} bytes; el máximo es ${n("max_bytes")}.`;
    case "vault_busy":
      return `El hub ya está leyendo ${n("in_flight")} peticiones del vault; espera a que terminen.`;
    case "vault_connection_fields":
      return `Revisa la URL, el vault y el token de Fast Note Sync.`;
    default:
      return Object.hasOwn(messages, code)
        ? (messages[code as VaultRefusalCode] ?? "")
        : `Orbit no reconoce este código de rechazo: ${code}.`;
  }
}

export type VaultRefusalCode = keyof typeof messages;
