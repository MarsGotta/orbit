// Several assistants in the app: «Tus asistentes», each one's conversation,
// the pencil, the gear, signing in, the plan's limit and «Añadir un asistente».
// Every text names the assistant by the name the person gave it; the service
// is named only where it is chosen or configured.
import { assistantsAccountCopy } from "./assistantsAccount";
import { hourOf, shortDay, usageWindow, vendorOf, whenBack } from "./assistantsFormat";
import { assistantsGearCopy } from "./assistantsGear";

export { hourOf, shortDay, vendorOf, whenBack };

/** «Hermes · API Gateway», «Codex · tu suscripción»: the service in one line. */
function serviceShort(service: string | undefined): string | undefined {
  switch (service) {
    case "hermes":
      return "Hermes · API Gateway";
    case "codex":
      return "Codex · tu suscripción";
    case "claude-code":
      return "Claude · tu suscripción";
    case "openai":
      return "Modelo de tu hub";
    default:
      return service;
  }
}

export const assistantsCopy = {
  // The column.
  title: "Asistente",
  listLabel: "Asistentes y conversaciones",
  yours: "Tus asistentes",
  add: "Añadir asistente",
  conversations: "Conversaciones",
  /** The service in one line, under the name and in the header's pill. */
  serviceShort,
  /** «Funciona con …», where it is chosen or configured. */
  serviceFull: (service: string | undefined): string | undefined => {
    switch (service) {
      case "hermes":
        return "Hermes, a través del API Gateway de tu hub";
      case "codex":
        return "Codex, con tu suscripción de ChatGPT";
      case "claude-code":
        return "Claude, con tu suscripción de Claude";
      case "openai":
        return "un modelo que configura tu hub";
      default:
        return service;
    }
  },
  worksWith: (line: string) => `Funciona con ${line}`,
  /** On the cards of «¿Con quién quieres hablar?»: «Codex · tu suscripción de ChatGPT». */
  serviceCard: (service: string | undefined): string | undefined => {
    const vendor = vendorOf(service);
    const short = serviceShort(service);
    return vendor === undefined || short === undefined ? short : `${short} de ${vendor}`;
  },
  serviceName: (service: string | undefined): string | undefined =>
    ({ hermes: "Hermes", codex: "Codex", "claude-code": "Claude", openai: "OpenAI" })[
      service ?? ""
    ] ?? service,
  // What goes wrong, at the right of its row.
  rowSignIn: "Inicia sesión",
  rowUntil: (atMs: number) => `Hasta ${hourOf(atMs)}`,
  rowLimit: "En su límite",
  // The conversation's row.
  rowLabel: (line: string, when: string, name: string | undefined) =>
    name === undefined ? `${line} · ${when}` : `${line} · ${when} · con ${name}`,

  // Its status, under its name.
  available: "Disponible",
  availableHub: "Disponible · con tu hub",
  availableUsage: (percent: number, window: string | undefined) =>
    `Disponible · ${percent} % del uso ${usageWindow(window)}`,
  signedOut: (vendor: string) => `Sin sesión iniciada en ${vendor}`,
  expired: (vendor: string) => `La sesión de ${vendor} caducó`,
  limitUntil: (atMs: number, nowMs: number) => `En su límite hasta ${whenBack(atMs, nowMs)}`,
  limitUnknown: "Ha llegado al límite de tu plan",
  principal: "Principal",

  // A new conversation.
  newConversation: "Conversación nueva",
  newConversationHint: "elige con quién hablar",
  chooseTitle: "¿Con quién quieres hablar?",
  chooseLead:
    "Cada asistente funciona con un servicio distinto. Puedes cambiar hasta que mandes el primer mensaje.",
  chooseWake: (primary: string | undefined, other: string | undefined) =>
    other === undefined
      ? primary === undefined
        ? "En una reunión también puedes llamarles por su nombre."
        : `En una reunión también puedes llamarle por su nombre: «Oye ${primary}, …».`
      : primary === undefined
        ? `En una reunión también puedes llamarles por su nombre: «Oye ${other}, …».`
        : `En una reunión también puedes llamarles por su nombre: «Oye ${other}, …». Sin nombre responde el principal, ${primary}.`,
  talkingTo: "Hablar con",

  // Each one's conversation.
  starters: "Para empezar",
  /** Three ideas that fill the box, by what the assistant works with. */
  startersFor: (service: string | undefined): readonly string[] => {
    switch (service) {
      case "codex":
        return [
          "Revisa un script y dime qué mejorarías",
          "Cruza dos tablas y dime qué no cuadra",
          "Explícame el último error del Terminal",
        ];
      case "claude-code":
        return [
          "Revisa este contrato antes de firmar",
          "Redacta una propuesta a partir de mis notas",
          "Resume este documento largo en una página",
        ];
      default:
        return [
          "¿Qué tengo hoy?",
          "Prepárame la próxima reunión",
          "Resume lo que quedó pendiente esta semana",
        ];
    }
  },
  continueWhere: "Seguir donde lo dejaste",
  noEarlier: (name: string) => `Aún no has hablado con ${name}.`,
  loadingEarlier: "Leyendo tus conversaciones…",

  // The header.
  editTip: "Editar nombre y descripción",
  editLabel: (name: string) => `Editar nombre y descripción de ${name}`,
  settingsTip: (name: string) => `Ajustes de ${name}`,
  limitState: (atMs: number | undefined) =>
    atMs === undefined ? "límite alcanzado" : `límite alcanzado · hasta las ${hourOf(atMs)}`,
  continuesFrom: (name: string) => `sigue desde ${name}`,

  // The pencil.
  editTitle: (name: string) => `Editar a ${name}`,
  name: "Nombre",
  nameHint: (name: string) => `Le llamas con «Oye ${name}».`,
  nameEmpty: "Ponle un nombre: es como le llamas.",
  nameTaken: "Ya tienes otro asistente que se llama así.",
  nameLong: (max: number) => `Como mucho ${max} caracteres.`,
  description: "Descripción",
  descriptionHint: "Se ve al elegir con quién hablar.",
  counter: (count: number, max: number) => `${count} / ${max}`,
  color: "Color",
  colors: ["Violeta", "Azul", "Verde", "Ámbar"] as const,
  primary: "Asistente principal",
  primaryIs: "Responde a «Oye…» sin nombre y en el panel flotante.",
  primaryKeep: "Para que deje de serlo, haz principal a otro asistente.",
  primaryWill: (current: string) =>
    `Responderá a «Oye…» sin nombre y en el panel flotante, en lugar de ${current}.`,
  primaryNow: (current: string) => `Ahora lo es ${current}.`,
  hubManaged:
    "Esto solo cambia cómo lo ves en Orbit. Su personalidad y lo que puede usar los gestiona tu hub.",
  moreSettings: "Más ajustes",
  saved: "✓ Guardado",
  ...assistantsAccountCopy,
  ...assistantsGearCopy,
} as const;
