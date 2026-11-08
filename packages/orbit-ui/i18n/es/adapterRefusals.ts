import { formatQuantity } from "../locale";
import { text, type Numbers } from "./refusalNumbers";

export function adapterRefusal(n: Numbers): string {
  const service = text(n, "service");
  if (!service) {
    const raw = text(n, "cause") ?? "adapter_unavailable";
    const key = /^[a-z_][a-z0-9_]*$/.test(raw) ? raw : "adapter_unavailable";
    switch (key) {
      case "missing":
        return "No se puede aislar al asistente: falta bubblewrap; instala bubblewrap en el hub";
      case "secret_scan_timeout":
        return `No se puede aislar al asistente todavía: la revisión de secretos del hub no terminó en ${formatQuantity(n.wait_limit_seconds)} segundos. Vuelve a intentarlo en unos instantes`;
      case "secret_scan_failed":
        return "No se puede aislar al asistente todavía: no se pudo completar la revisión de secretos del hub. Vuelve a intentarlo en unos instantes";
      case "secret_scan_not_ready":
        return "No se puede aislar al asistente todavía: la revisión de secretos del hub aún no ha terminado. Vuelve a intentarlo en unos instantes";
      default:
        return `No se puede crear la jaula de bubblewrap (${key}); revisa los espacios de nombres de usuario y el perfil de AppArmor de Ubuntu (bwrap-userns-restrict) para permitir bubblewrap`;
    }
  }
  const vendor = service === "claude-code" ? "Claude" : service === "codex" ? "Codex" : service;
  const causes: Record<string, string> = {
    adapter_path_missing: "no existe",
    adapter_missing: "no existe el ejecutable o su intérprete en esta plataforma",
    adapter_not_file: "no es un archivo",
    adapter_permission: "no tiene permiso de ejecución",
    adapter_format: "el binario no se puede ejecutar en esta plataforma",
    adapter_launch_failed: "el sistema rechazó el arranque",
    adapter_unavailable: "no está disponible",
  };
  const code = text(n, "cause_code") ?? "adapter_unavailable";
  const cause = Object.hasOwn(causes, code) ? causes[code] : `causa ${code}`;
  const path = text(n, "path");
  const detail = path ? `: ${path} (${cause})` : "";
  return `${vendor} no está disponible en tu hub${detail}. Publica de nuevo el hub para instalarlo.`;
}
