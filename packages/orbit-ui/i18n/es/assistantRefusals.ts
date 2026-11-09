import { adapterRefusal } from "./adapterRefusals";
import { numberFormat } from "../locale";
import { count, text, type Numbers } from "./refusalNumbers";

/**
 * The words of the hub's refusals of an assistant's profile, of adding and
 * removing one and of signing it in and out (hub-v1 «Assistants from
 * Orbit»); `undefined` when a number they need is missing.
 */
export const assistantRefusals: Record<string, (n: Numbers) => string | undefined> = {
  agent_profile_invalid: (n) => {
    const field = text(n, "field");
    const rule = text(n, "rule");
    const what =
      field === "description" ? "La descripción" : field === "color" ? "El color" : "El nombre";
    if (rule === "empty")
      return field === "name" ? "Ponle un nombre: es como le llamas." : undefined;
    if (rule === "too_long") {
      const characters = count(n, "characters");
      const max = count(n, "max_characters");
      if (characters === undefined || max === undefined) return undefined;
      return `${what} tiene ${numberFormat().format(characters)} caracteres; el máximo es ${numberFormat().format(max)}.`;
    }
    if (rule === "control_characters")
      return `${what} lleva caracteres de control; ${field === "description" ? "escríbela" : "escríbelo"} en una línea.`;
    if (rule === "unknown_color") return "Ese color no es uno de los cuatro de Orbit.";
    return undefined;
  },
  agent_name_taken: () => "Ya tienes otro asistente que se llama así.",
  agent_primary_required: () =>
    "Siempre hay un asistente principal: haz principal a otro para que este deje de serlo.",
  agent_add_unsupported: () =>
    "Este tipo se añade en tu hub; Hermes se crea a partir de un perfil de Hermes.",
  agent_adapter_missing: adapterRefusal,
  too_many_agents: (n) => {
    const agents = count(n, "count");
    const max = count(n, "max");
    if (agents === undefined || max === undefined) return undefined;
    return `Tu hub ya tiene ${numberFormat().format(agents)} asistentes; el máximo es ${numberFormat().format(max)}.`;
  },
  agent_not_removable: () => "Este asistente lo da tu hub: solo quien lo gestiona puede quitarlo.",
  agent_folder_not_removed: (n) => {
    const errno = count(n, "errno");
    if (errno === undefined) return undefined;
    return `El hub no pudo borrar la carpeta del asistente (errno ${numberFormat().format(errno)}), así que sigue aquí. Avisa a quien gestiona tu hub.`;
  },
  sign_in_unsupported: () =>
    "Este asistente no inicia sesión desde Orbit: lo configura quien gestiona tu hub.",
  sign_in_not_found: () =>
    "El hub ya no tiene ese inicio de sesión: caducó o terminó. Vuelve a empezarlo.",
  sign_in_not_waiting_for_code: () =>
    "El inicio de sesión ya no espera un código. Vuelve a empezarlo.",
  sign_in_code_invalid: (n) => {
    const characters = count(n, "characters");
    if (characters === undefined) return undefined;
    return `Ese código no es el de la página (${numberFormat().format(characters)} caracteres). Cópialo entero, con su «#».`;
  },
  sign_out_failed: (n) => {
    const status = count(n, "exit_status");
    if (status === undefined) return undefined;
    return `La herramienta del proveedor no cerró la sesión (código de salida ${numberFormat().format(status)}). Vuelve a intentarlo.`;
  },
  sign_out_timeout: (n) => {
    const timeout = count(n, "timeout_ms");
    if (timeout === undefined) return undefined;
    return `La herramienta del proveedor no cerró la sesión en ${numberFormat().format(Math.round(timeout / 1000))} s. Vuelve a intentarlo.`;
  },
};
