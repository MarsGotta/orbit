import { numberFormat } from "../locale";
// The window: the sidebar with its recording and call cards, the top bar
// with its notices, and what the window says before Orbit answers.
import { duration } from "./duration";

/** «hasta las 12:00», and «hasta la 1:00» for one o'clock. */
function until(time: string): string {
  return time.startsWith("1:") ? `hasta la ${time}` : `hasta las ${time}`;
}

function sidebarCopy(assistantName: string) {
  return {
    shell: {
      /** The agent every text names by default: the open conversation's. */
      assistantName,
      viewsLabel: "Principal",
      breadcrumbsLabel: "Dónde estás",
      record: "Grabar reunión",
      recordShortcut: "⌃⌥M",
      briefing: "Hoy",
      calendar: "Calendario",
      notes: "Reuniones",
      vault: "Notas",
      assistant: "Asistente",
      settings: "Ajustes",
      inProgress: (count: number) => `${numberFormat().format(count)} en curso`,
      needsYou: "Te necesita",
      localUser: "Tú",
      ask: `Preguntar a ${assistantName}`,
      search: `Buscar o preguntar a ${assistantName}`,
      searchShortcut: "⌘⇧␣",
      changeTheme: "Cambiar tema",
      toLight: "Cambiar a tema claro",
      toDark: "Cambiar a tema oscuro",
      loadingView: "Cargando…",
      notice: "Aviso de Orbit",
      closeNotice: "Cerrar aviso",
      settingsSections: {
        general: "General",
        privacidad: "Privacidad y permisos",
        grabacion: "Grabación y llamadas",
        asistentes: "Asistentes",
        reuniones: "Reuniones",
        notas: "Notas",
        ia: "IA",
      },
    },
  } as const;
}

function recordingCopy(assistantName: string) {
  return {
    recordingCard: {
      label: "Grabación en curso",
      starting: "Preparando la grabación…",
      recording: "Grabando",
      paused: "En pausa",
      finishing: "Guardando la grabación…",
      elapsed: "Tiempo grabado",
      audio: {
        system: "Audio del sistema",
        microphone: "Solo tu micrófono",
        application: "Audio de una app",
      },
      audioOf: (application: string) => `Audio de ${application}`,
      until,
      listening: `«Oye ${assistantName}» te escucha`,
      notInPause: `«Oye ${assistantName}» no escucha en pausa`,
      ask: `Preguntar a ${assistantName}`,
      pause: "Pausar",
      resume: "Seguir",
      finish: "Terminar",
      check: "Revisar",
    },
    askMenu: {
      title: "Pregunta sobre esta reunión",
      fieldLabel: "Tu pregunta",
      placeholder: "¿Qué se decidió sobre el presupuesto?",
      windows: "Escucha los últimos",
      window: (minutes: number) => `${numberFormat().format(minutes)} min`,
      voiceHint: `O di «Oye ${assistantName}» en voz alta`,
      send: "Preguntar",
      close: "Cerrar",
      alsoInConversation: `Queda también en tu conversación con ${assistantName}`,
      another: "Otra pregunta",
    },
  } as const;
}

const callCopy = {
  callCard: {
    label: "Llamada en curso",
    inCall: "En llamada",
    elapsed: "Tiempo de llamada",
    seesScreen: "Ve tu pantalla",
    noScreen: "No ve tu pantalla",
    left: (minutes: number) => (minutes <= 1 ? "queda 1 min" : `quedan ${duration(minutes)}`),
    hangUp: "Colgar",
  },
} as const;

const noticesCopy = {
  // The top bar says nothing when all is well; otherwise the most
  // important thing, one at a time, with the button that fixes it.
  // Nothing here names a server, a device or a count.
  problem: {
    orbitLost: "Orbit no responde",
    privacyStopped: "Todo está parado",
    unpaired: "Este Mac no está emparejado",
    revoked: "El hub retiró este Mac",
    offline: "Sin conexión con el hub",
    missingPermission: (name: string) => `Falta el permiso de ${name}`,
    missingPermissions: "Faltan permisos",
    retrying: "Reintentando…",
    connectingHub: "Conectando con tu hub…",
    preparing: "Orbit se está preparando…",
    pair: "Emparejar",
    fix: "Arreglar",
    rearm: "Reactivar",
    rearming: "Reactivando…",
    more: (count: number) => `+${numberFormat().format(count)}`,
    moreLabel: (count: number) =>
      count === 1 ? "Ver 1 aviso más" : `Ver ${numberFormat().format(count)} avisos más`,
    all: "Avisos",
  },
  appState: {
    starting: "Conectando con Orbit…",
    startingHint: (seconds: number) =>
      `Si tarda más de ${numberFormat().format(seconds)} segundos, te diremos qué pasa.`,
    slow: "Orbit está tardando más de lo normal",
    slowText:
      "Puede que se haya quedado bloqueado. Ciérralo y vuelve a abrirlo: tus grabaciones guardadas están a salvo.",
    lost: "Orbit no responde",
    lostText: "Ciérralo y vuelve a abrirlo. Tus grabaciones guardadas están a salvo.",
    close: "Cerrar Orbit",
  },
} as const;

export function shellCopy(assistantName: string) {
  return {
    ...sidebarCopy(assistantName),
    ...recordingCopy(assistantName),
    ...callCopy,
    ...noticesCopy,
  } as const;
}
