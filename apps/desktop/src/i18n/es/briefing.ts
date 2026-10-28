import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { listFormat } from "../locale";
import { duration } from "./duration";
import { globalShortcuts } from "./shortcuts";

const pendingLabel = "lo pendiente de la última vez";

const base = {
  briefing: {
    morning: "Buenos días",
    afternoon: "Buenas tardes",
    night: "Buenas noches",
    // Without the Mac's zone the hour of the day is not known.
    hello: "Hola",
    week: (week: number) => `semana ${numberFormat().format(week)}`,
    daySentence: (meetings: number, focusMinutes: number) => {
      if (meetings === 0) return "Hoy no tienes reuniones.";
      const count =
        pluralRules().select(meetings) === "one"
          ? "1 reunión"
          : `${numberFormat().format(meetings)} reuniones`;
      return focusMinutes === 0
        ? `Hoy tienes ${count} y ningún hueco libre para concentrarte.`
        : `Hoy tienes ${count} y ${duration(focusMinutes)} libres para concentrarte.`;
    },
    nextStartsIn: (minutes: number) => `Siguiente · empieza en ${duration(minutes)}`,
    nextNow: "Ahora · en curso",
    videoCall: "videollamada",
    withPeople: (names: string[]) => `Con ${listFormat({ type: "conjunction" }).format(names)}`,
    previousTitle: (title: string) => `Nota anterior: ${title}`,
    joinRecord: "Unirse y grabar",
    // An event with no video call has nothing to join.
    onlyRecord: "Solo grabar",
    previousNote: "Abrir nota anterior",
    pending: "Pendiente de la última vez",
    pendingReading: `Leyendo ${pendingLabel}…`,
    pendingFailed: `No se pudo leer ${pendingLabel}.`,
    pendingFirst: "Es la primera de esta serie: no hay nada pendiente.",
    pendingNone: "No quedó nada pendiente.",
    pendingNoNote: "La última vez no se escribió nota: no se sabe qué quedó pendiente.",
    pendingWriting: "La nota de la última vez aún se está escribiendo.",
    pendingNoteFailed: "La nota de la última vez no se pudo escribir.",
    pendingTask: (owner: string | null, text: string) =>
      owner === null || owner === "" ? text : `${owner}: ${text}`,
    reminder: "Recordatorio personal",
    reminderPlaceholder: "Algo que no quieras olvidar…",
    reminderHelp: "Solo lo ves tú. No va a la nota.",
    reminderNotKept: (cause: "quota" | "unavailable") =>
      cause === "quota"
        ? "No se guardó: no queda espacio en este Mac para los datos locales de Orbit. Lo que escribiste sigue aquí."
        : "No se guardó: este Mac no deja a Orbit guardar datos locales ahora. Lo que escribiste sigue aquí.",
    recordingClock: (clock: string) => `Grabando · ${clock}`,
    paused: "En pausa",
    recordingHint:
      "Mientras grabas, apunta lo que no quieras olvidar. Orbit lo usará para escribir la nota.",
    recordsUntil: (clock: string) => `Graba hasta las ${clock}`,
    privateNotes: "Apuntar en mis notas privadas",
    finish: "Terminar",
    during: "Durante la reunión",
    askAloud: "Preguntar en voz alta",
    askSilently: "Preguntar sin hablar",
    pttShortcut: globalShortcuts.ptt,
    finishAnywhere: "Terminar desde cualquier app",
    // The helper's `meeting_toggle` (GlobalHotkeys.swift).
    meetingShortcut: "⌃⌥M",
    agenda: "Agenda de hoy",
    seeWeek: "Ver la semana",
    dayLine: (now: string | null) =>
      now === null ? "Tu día de 8:00 a 20:00." : `Tu día de 8:00 a 20:00. Ahora son las ${now}.`,
    noteReady: "Nota lista",
    recording: "Grabando",
    peopleCount: (count: number) =>
      count === 0
        ? "Solo tú"
        : pluralRules().select(count) === "one"
          ? "1 persona"
          : `${numberFormat().format(count)} personas`,
    readingAgenda: "Leyendo tu agenda…",
    agendaFailed: "No se pudo leer tu agenda.",
    noMeetingsTitle: "Nada en tu agenda hoy",
    noneLeftTitle: "No te quedan reuniones hoy",
    noMeetingsText: "Si surge una, grábala y Orbit escribirá la nota por ti.",
    record: "Grabar reunión",
    calendarTitle: "Conecta tu calendario",
    calendarText:
      "Orbit usa los calendarios de tu Mac (iCloud, Google, Exchange…) y no guarda una copia.",
    calendarAllow: "Permitir acceso al calendario",
    calendarHint: "¿Lo denegaste antes? Actívalo en Ajustes del Sistema.",
    calendarDenied: "Orbit no tiene acceso a tus calendarios. Actívalo en Ajustes del Sistema.",
    openSystemSettings: "Abrir Ajustes del Sistema",
    // What stops the agenda when the permission is there; the top bar's
    // notice carries the button that fixes it.
    blocked: {
      privacy_stopped: "Todo está parado: Orbit no lee tu agenda hasta que lo reactives.",
      helper_not_running:
        "Orbit no puede leer tu agenda ahora: la parte que la lee no está en marcha.",
      not_paired: "Para ver tu agenda, empareja este Mac con tu hub.",
      mac_disconnected: "Este Mac no está conectado: tu agenda se verá cuando vuelva.",
    },
    timezoneFailed:
      "No se pudo leer la zona horaria del Mac, así que Hoy no puede situar tus reuniones.",
  },
} as const;

export function briefingCopy(assistantName: string) {
  return { briefing: { ...base.briefing, heyDeimos: `«Oye ${assistantName}…»` } } as const;
}
