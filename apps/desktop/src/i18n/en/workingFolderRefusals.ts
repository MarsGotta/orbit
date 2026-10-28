import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

export function workingFolderRefusal(n: Numbers): string {
  const code = String(n.cause_code ?? "working_folder_legacy");
  const causes: Record<string, string> = {
    working_folder_home: "cannot be the hub user’s home folder or an ancestor of it",
    working_folder_installed: "overlaps installed hub code or its Python environment",
    working_folder_reserved: "includes the reserved root for assistant folders",
    working_folder_protected: "overlaps protected data, credentials or another assistant’s folder",
    working_folder_absolute: "must be an absolute hub path",
    working_folder_directory: "is not a directory on the hub",
    working_folder_unavailable: "does not exist or cannot be opened on the hub",
    working_folder_unsupported: "this assistant does not use a hub working folder",
  };
  if (code === "working_folder_size")
    return `The working folder must use at most ${formatQuantity(n.max_bytes)} bytes and contain no NUL (${formatQuantity(n.bytes)} bytes).`;
  return Object.hasOwn(causes, code)
    ? `The working folder ${causes[code]}`
    : `The hub refused the working folder (${code}).`;
}
