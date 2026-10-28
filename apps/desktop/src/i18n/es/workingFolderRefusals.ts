import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

export function workingFolderRefusal(n: Numbers): string {
  const code = String(n.cause_code ?? "working_folder_legacy");
  const causes: Record<string, string> = {
    working_folder_home:
      "no puede ser la carpeta personal del usuario del hub ni una de sus antecesoras",
    working_folder_installed: "se solapa con el código instalado del hub o su entorno de Python",
    working_folder_reserved: "incluye la raíz reservada para las carpetas de los asistentes",
    working_folder_protected:
      "se solapa con datos o credenciales protegidos, o con la carpeta de otro asistente",
    working_folder_absolute: "debe ser una ruta absoluta del hub",
    working_folder_directory: "no es un directorio del hub",
    working_folder_unavailable: "no existe o no se puede abrir en el hub",
    working_folder_unsupported: "este asistente no usa una carpeta de trabajo del hub",
  };
  if (code === "working_folder_size")
    return `La carpeta de trabajo debe ocupar como máximo ${formatQuantity(n.max_bytes)} bytes y no contener NUL (${formatQuantity(n.bytes)} bytes).`;
  return Object.hasOwn(causes, code)
    ? `La carpeta de trabajo ${causes[code]}`
    : `El hub rechazó la carpeta de trabajo (${code}).`;
}
