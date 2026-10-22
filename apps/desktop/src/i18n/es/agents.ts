import type { AccessGroups } from "../../bridge/generated/AccessGroups";
import type { AgentRefusal } from "../../bridge/generated/AgentRefusal";
import type { RefusalNumber } from "../../bridge/generated/RefusalNumber";

/**
 * The groups of Orbit's tools an agent may use, in the order Ajustes shows
 * them (hub-v1 «Agents»), each said by what it lets the agent do. The tools
 * each one opens are the hub's (`TOOLS` in `server/orbit_hub/assistant/access.py`).
 */
export const accessGroups: ReadonlyArray<{
  group: keyof AccessGroups;
  label: string;
  description: string;
}> = [
  {
    group: "screen",
    label: "Pantalla",
    description: "Ver tus ventanas, hacer capturas y leer el texto que selecciones",
  },
  { group: "control", label: "Usar el Mac", description: "Pulsar, escribir y abrir apps por ti" },
  { group: "calendar", label: "Calendario", description: "Leer tu agenda" },
  {
    group: "meetings",
    label: "Reuniones y notas",
    description: "Leer tus reuniones, notas y transcripciones",
  },
  {
    group: "files",
    label: "Archivos",
    description: "Leer los archivos de las carpetas que elijas",
  },
];

type Numbers = AgentRefusal["numbers"];

/** A count as the person reads it: «16.384». */
const figure = new Intl.NumberFormat("es-ES").format;

function text(numbers: Numbers, key: string): string | undefined {
  const value = numbers[key];
  return typeof value === "string" ? value : undefined;
}

function count(numbers: Numbers, key: string): number | undefined {
  const value = numbers[key];
  return typeof value === "number" ? value : undefined;
}

/** Every number as `key: value`, for a refusal this screen has no words for. */
function listed(numbers: Numbers): string {
  return Object.entries(numbers)
    .filter((entry): entry is [string, RefusalNumber] => entry[1] !== undefined)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}: ${String(value)}`)
    .join(", ");
}

/** The words of each refusal; `undefined` when a number it needs is missing. */
const refusals: Record<string, (numbers: Numbers) => string | undefined> = {
  agent_not_found: (n) => {
    const agent = text(n, "agent_id");
    if (agent === undefined) return undefined;
    const known = text(n, "agent_ids");
    return `El hub no tiene el agente «${agent}»${known ? `; tiene ${known}` : ""}.`;
  },
  skill_delete_unsupported: () =>
    "El panel de Hermes no puede borrar habilidades: desactívala aquí o bórrala en el servidor de Hermes.",
  skill_invalid: (n) => {
    const name = text(n, "name");
    const rule = text(n, "rule");
    if (name === undefined || rule === undefined) return undefined;
    return `La habilidad «${name}» no es válida: no cumple la regla «${rule}».`;
  },
  skill_not_found: (n) => {
    const name = text(n, "name");
    return name === undefined ? undefined : `El hub no tiene la habilidad «${name}».`;
  },
  skill_too_large: (n) => {
    const name = text(n, "name");
    const bytes = count(n, "bytes");
    const max = count(n, "max_bytes");
    if (name === undefined || bytes === undefined || max === undefined) return undefined;
    return `La habilidad «${name}» ocupa ${figure(bytes)} bytes; el máximo es ${figure(max)}.`;
  },
  too_many_skills: (n) => {
    const skills = count(n, "count");
    const max = count(n, "max");
    if (skills === undefined || max === undefined) return undefined;
    return `Este agente ya tiene ${skills} habilidades; el máximo es ${max}.`;
  },
  personality_too_large: (n) => {
    const bytes = count(n, "bytes");
    const max = count(n, "max_bytes");
    if (bytes === undefined || max === undefined) return undefined;
    return `La personalidad ocupa ${figure(bytes)} bytes; el máximo es ${figure(max)}.`;
  },
  assistant_admin_refused: (n) => {
    const route = text(n, "route");
    const status = count(n, "status");
    if (route === undefined || status === undefined) return undefined;
    return `Hermes no aceptó el cambio: ${route} respondió HTTP ${status}.`;
  },
};

/** The hub's refusal of a read or an order, with its cause and numbers. */
export function agentRefusalText(refusal: AgentRefusal): string {
  const words = Object.hasOwn(refusals, refusal.code)
    ? refusals[refusal.code]?.(refusal.numbers)
    : undefined;
  if (words !== undefined) return words;
  const numbers = listed(refusal.numbers);
  return `El hub rechazó la orden (${refusal.code}, HTTP ${refusal.status})${numbers ? `: ${numbers}` : ""}.`;
}

/**
 * Why the Mac could not ask the hub, or could not read its answer: the
 * daemon's typed refusal (`Refusal`), with its numbers.
 */
export function agentFailureText(refusal: Record<string, unknown>): string {
  const status = typeof refusal.status === "number" ? refusal.status : null;
  switch (refusal.code) {
    case "not_paired":
      return "Este Mac no está emparejado con el hub. Emparéjalo en Ajustes.";
    case "privacy_stopped":
      return "Todo está parado. Reactiva Orbit para leer o cambiar los agentes.";
    case "hub_unreachable":
      return "No hay conexión con el hub; los agentes se leen y se cambian en él.";
    case "upstream_failed":
      if (status === 404)
        return "El hub respondió HTTP 404 sin código: puede que su versión todavía no tenga la configuración de agentes.";
      return status === null
        ? "El hub no dio su respuesta: la conexión se cortó o no respondió a tiempo. Vuelve a leer para ver si el cambio se aplicó."
        : `El hub respondió HTTP ${status}.`;
    case "hub_reply_invalid":
      return `La respuesta del hub no cumple el contrato (campo ${String(refusal.field)}).`;
    case "too_large":
      return `La respuesta del hub ocupa ${String(refusal.bytes)} bytes; el máximo es ${String(refusal.max_bytes)}.`;
    default:
      return `Orbit no pudo preguntar al hub (${String(refusal.code)}).`;
  }
}

export const agents = {
  agents: {
    section: "Agentes",
    none: "Todavía no se conocen los agentes del hub: aparecen al conectar con él.",
    unavailable: "Esta ventana no puede leer los agentes.",
    loading: (name: string) => `Leyendo la configuración de ${name}…`,
    readAgain: "Volver a leer",
    // How the agent is configured, by its kind (hub-v1 «Agents»).
    kind: (kind: string, name: string) =>
      kind === "hermes"
        ? `${name} funciona con Hermes: su personalidad y lo que puede usar se cambian en su perfil de Hermes.`
        : kind === "openai" || kind === "acp"
          ? `${name} se configura desde aquí. Los cambios se guardan en el hub.`
          : `${name} es de un tipo que esta versión de Orbit no conoce (${kind}).`,
    notEditable: (cause: string | null | undefined) =>
      cause
        ? `No se puede cambiar desde Orbit: ${cause}`
        : "Este tipo de agente no deja cambiarlo desde Orbit.",
    saving: "Guardando en el hub…",
    saved: "✓ Guardado en el hub",
    bytes: (bytes: number, max: number) => `${figure(bytes)} de ${figure(max)} bytes`,
    overLimit: (bytes: number, max: number) =>
      `Ocupa ${figure(bytes)} bytes; el máximo es ${figure(max)}.`,
    personality: "Personalidad",
    personalityLabel: (name: string) => `Personalidad de ${name}`,
    savePersonality: "Guardar personalidad",
    access: "Qué puede usar",
    accessHelp:
      "macOS sigue decidiendo sobre este Mac; esto solo limita a este agente. No pregunta en cada uso ni caduca.",
    skills: "Habilidades",
    skillCount: (skills: number, max: number) => `${skills} de ${max}`,
    noSkills:
      "Este agente aún no tiene habilidades. Una habilidad le enseña a hacer una tarea concreta, como preparar una reunión.",
    enable: (name: string) => `Activar ${name}`,
    view: (name: string) => `Ver ${name}`,
    edit: (name: string) => `Editar ${name}`,
    viewLabel: "Ver",
    editLabel: "Editar",
    delete: (name: string) => `Borrar ${name}`,
    deleteLabel: "Borrar",
    deleteQuestion: (name: string) => `¿Borrar ${name} de este agente? No se puede deshacer.`,
    confirmDelete: "Confirmar borrado",
    cancel: "Cancelar",
    newSkill: "Nueva habilidad",
    paste: "Escribir desde cero",
    importFolder: "Importar carpeta",
    folderInput: "Carpeta con un SKILL.md",
    importHelp:
      "Una habilidad es un SKILL.md con name y description en su frontmatter. Al importar una carpeta solo se lee y se envía su SKILL.md.",
    noSkillFile: (folder: string) => `La carpeta «${folder}» no tiene un SKILL.md.`,
    skillFileTooLarge: (folder: string, bytes: number, max: number) =>
      `El SKILL.md de «${folder}» ocupa ${figure(bytes)} bytes; el máximo es ${figure(max)}.`,
    skillFileUnreadable: (folder: string) => `No se pudo leer el SKILL.md de «${folder}».`,
    content: "Contenido del SKILL.md",
    name: (name: string) => `Nombre: ${name}`,
    noName: "Falta el name en el frontmatter del SKILL.md.",
    saveSkill: "Guardar habilidad",
    close: "Cerrar",
    template: "---\nname: \ndescription: \n---\n\n",
  },
} as const;
