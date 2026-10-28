import { schemaProblemText } from "./schemaRefusalDetails";
import { vaultRequestRefusal } from "./vaultRequestRefusals";
import { formatQuantity } from "../locale";

const messages: Record<import("../es/vaultRefusals").VaultRefusalCode, string> = {
  vault_folder_absolute: "Enter an absolute path to a folder on the hub.",
  vault_folder_missing: "The vault folder does not exist on the hub.",
  vault_folder_permission: "The hub does not have permission to read the vault folder.",
  vault_path_refused:
    "The path must stay inside the vault, without hidden files or symbolic links.",
  vault_note_missing: "The note does not exist in the vault.",
  vault_not_markdown: "Only Markdown notes (.md) can be read.",
  vault_encoding: "The note is not encoded as UTF-8.",
  vault_folder_changed: "A folder changed or cannot be read; search again.",
  vault_sync_connection: "Could not connect to Fast Note Sync; check the URL and service.",
  vault_sync_shape: "Fast Note Sync returned an invalid response format.",
  vault_sync_missing: "Fast Note Sync cannot find that vault.",
  vault_token_refused: "Fast Note Sync rejected the token; check its validity and vault access.",
  vault_token_expired: "The Fast Note Sync token has expired.",
  vault_sync_refused: "Fast Note Sync refused the vault read.",
  vault_not_connected: "Connect a hub folder or Fast Note Sync in Notes.",
  vault_frontmatter_invalid: "The note’s YAML frontmatter is invalid.",
  vault_path_controls: "The vault path contains control characters.",
  vault_path_components: "The folder cannot contain “.” or “..” components.",
  vault_path_symlink: "The folder or note cannot be opened without following symbolic links.",
  vault_path_regular_file: "The note must be a regular file in the vault.",
  vault_sync_note_missing: "The note does not exist in Fast Note Sync.",
  vault_sync_token_required: "Fast Note Sync requires a valid token.",
  vault_sync_list_shape: "The Fast Note Sync list has an unexpected format.",
  vault_sync_note_shape: "Fast Note Sync returned a note with a different format or path.",
  vault_sync_links_shape: "Fast Note Sync returned invalid links.",
  vault_sync_link_path: "Fast Note Sync returned a link without a path.",
};

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
  if (schema !== undefined) return `The note YAML frontmatter: ${schema}.`;
  const n = (key: string) => formatQuantity(numbers[key]);
  if (code === "vault_invalid_request") return vaultRequestRefusal(numbers, fields);
  switch (code) {
    case "vault_sync_catalog_incomplete":
      return `The Fast Note Sync catalogue does not finish within the limit.`;
    case "vault_sync_no_response":
      return `Fast Note Sync did not respond within ${n("timeout_seconds")} seconds.`;
    case "vault_sync_response_too_large":
      return `The Fast Note Sync response exceeds ${n("max_bytes")} bytes.`;
    case "vault_sync_scan_limit":
      return `The vault exceeds ${n("max_entries")} entries per scan.`;
    case "vault_sync_links_limit":
      return `The note exceeds ${n("max_entries")} links.`;
    case "vault_index_scan_bytes":
      return `The index exceeds ${n("max_bytes")} bytes read per scan.`;

    case "vault_note_too_large":
      return `The note is ${n("bytes")} bytes; the maximum is ${n("max_bytes")}.`;
    case "vault_scan_limit":
      return `The scan exceeds ${n("max_entries")} entries.`;
    case "vault_depth_limit":
      return `The vault exceeds ${n("max_depth")} folder levels.`;
    case "vault_scan_bytes":
      return `The search exceeds ${n("max_bytes")} bytes read; narrow the folder.`;
    case "vault_sync_timeout":
      return `The Fast Note Sync read exceeded ${n("timeout_seconds")} seconds.`;
    case "vault_sync_http":
      return `Fast Note Sync returned HTTP ${n("upstream_status")}.`;
    case "vault_response_too_large":
      return `The Notes response is ${n("bytes")} bytes; the maximum is ${n("max_bytes")}.`;
    case "vault_busy":
      return `The hub is already reading ${n("in_flight")} vault requests; wait for them to finish.`;
    case "vault_connection_fields":
      return `Check the Fast Note Sync URL, vault and token.`;
    default:
      return Object.hasOwn(messages, code)
        ? (messages[code as import("../es/vaultRefusals").VaultRefusalCode] ?? "")
        : `Orbit does not recognise this refusal code: ${code}.`;
  }
}
