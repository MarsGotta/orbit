import { hourOf, sameDay, usageWindow, weekday } from "./assistantsFormat";

// An assistant's settings, behind its gear, and «Añadir un asistente».
export const assistantsGearCopy = {
  // The gear.
  backTo: (name: string) => `Conversación con ${name}`,
  settingsCrumb: "Ajustes",
  settingsTitle: (name: string) => `Ajustes de ${name}`,
  settingsLead:
    "El nombre, la descripción, el color y si es el principal se cambian con el lápiz. Aquí está el resto.",
  connection: "Conexión",
  service: "Servicio",
  where: "Dónde",
  whereHub: (hub: string) => `API Gateway de ${hub}`,
  account: "Cuenta",
  state: "Estado",
  stateConnected: "Conectado",
  stateExpired: "Sesión caducada",
  stateSignedOut: "Sin sesión",
  stateUnknown: "Sin comprobar todavía",
  stateLimit: (atMs: number | undefined) =>
    atMs === undefined
      ? "En el límite de tu plan"
      : `En el límite de tu plan hasta las ${hourOf(atMs)}`,
  hubConfigured:
    "Lo configura quien gestiona tu hub. Su personalidad y lo que puede usar se cambian en Hermes; aquí lo ves tal como está.",
  usage: "Uso de tu plan",
  usageLine: (percent: number, window: string | undefined) =>
    `${percent} % del uso ${usageWindow(window, true)}`,
  renews: (atMs: number, nowMs: number) => {
    const at = new Date(atMs);
    if (sameDay(at, new Date(nowMs))) return `Se renueva hoy a las ${hourOf(atMs)}`;
    return `Se renueva el ${weekday.format(at)}`;
  },
  usageHint: "Si llega al límite, Orbit te ofrecerá seguir con otro asistente.",
  seePlan: (vendor: string) => `Ver mi plan de ${vendor}`,
  signOut: "Cerrar sesión",
  signedOutDone: (name: string) =>
    `Has cerrado la sesión. ${name} sigue en «Tus asistentes» con sus conversaciones; vuelve a iniciarla para escribirle.`,
  expiredBanner: (vendor: string) => `La sesión de ${vendor} caducó`,
  expiredBannerText: (name: string) =>
    `Inicia sesión otra vez para que ${name} pueda responder. Sus conversaciones siguen aquí.`,
  inOrbit: "En Orbit",
  inMeeting: "En una reunión",
  inMeetingText: (name: string) => `Di «Oye ${name}, …» y te responde en el panel flotante.`,
  voice: "Voz",
  ownVoice: "Tiene voz propia: la usa en las llamadas y al escuchar sus respuestas.",
  borrowedVoice: (primary: string) =>
    `No tiene voz propia: Orbit lee sus respuestas con la voz de ${primary}.`,
  history: "Lo que han hecho tus asistentes en este Mac",
  historyShared: "Tu hub todavía no dice qué asistente hizo cada cosa: aquí está lo de todos.",
  remove: (name: string) => `Quitar a ${name} de Orbit`,
  removeQuestion: (name: string) => `¿Quitar a ${name}?`,
  removeText:
    "Sus conversaciones se quedan, pero ya no podrás escribirle. Tu suscripción no cambia.",
  removeConfirm: "Quitar",
  removing: "Quitando…",
  notRemovable: (name: string) => `${name} lo da tu hub: solo quien lo gestiona puede quitarlo.`,
  unknownAssistant: "Ese asistente ya no está en tu hub.",
  backToConversation: "Volver a la conversación",

  // «Añadir un asistente».
  addStep: (step: number) => `Añadir un asistente · paso ${step} de 4`,
  kindTitle: "¿Con qué va a funcionar?",
  kindLead: "Cada asistente usa un servicio. Si ya pagas uno, puedes usar esa suscripción.",
  kindLabel: "Servicio",
  kinds: {
    "claude-code": {
      title: "Tu suscripción de Claude",
      text: "Inicia sesión con tu cuenta de Claude (Pro o Max). Usa el uso de tu plan.",
      tag: "Claude",
    },
    codex: {
      title: "Tu suscripción de ChatGPT",
      text: "Inicia sesión con tu cuenta de ChatGPT (Plus o Pro) para usar Codex.",
      tag: "Codex",
    },
    hermes: {
      title: "Hermes en tu hub",
      text: "Lo prepara quien gestiona tu hub, a través del API Gateway de tu empresa.",
      tag: "Hermes",
    },
  } as Record<string, { title: string; text: string; tag: string }>,
  notOffered: "Tu hub no tiene instalado este servicio. Pídeselo a quien gestiona tu hub.",
  connectTitle: (vendor: string) => `Conecta tu cuenta de ${vendor}`,
  connectLead: "Orbit usará tu plan para este asistente. Lo que gastes cuenta en tu suscripción.",
  connectHint:
    "Se abre tu navegador. Orbit no ve tu contraseña: solo recibe permiso para usar tu plan.",
  hubKindTitle: "Un asistente de tu hub",
  hubKindLead: "Este tipo no se crea desde Orbit.",
  askHub: "Pídeselo a quien gestiona tu hub",
  askHubText:
    "Los asistentes con Hermes se crean en el hub (por ejemplo, detrás del API Gateway de tu empresa). Cuando lo añadan, aparecerá aquí solo.",
  preparing: "Preparando el asistente en tu hub…",
  nameTitle: "Ponle nombre",
  nameLead: (name: string) => `Así le llamarás en Orbit y en las reuniones («Oye ${name}, …»).`,
  accessTitle: "Qué puede usar · podrás cambiarlo en sus ajustes",
  create: "Crear asistente",
  creating: "Creando…",
  readyTitle: (name: string) => `${name} está listo`,
  readyText: (name: string) =>
    `Aparece en «Tus asistentes». Háblale desde una conversación nueva o di «Oye ${name}, …» en una reunión.`,
  talkTo: (name: string) => `Hablar con ${name}`,
  back: "Atrás",
  next: "Continuar",
  close: "Cerrar",
} as const;
