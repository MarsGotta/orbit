import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { macPermissions } from "./permissions";
import { settingsLists } from "./settingsLists";
import { globalShortcuts } from "./shortcuts";

const base = {
  settings: {
    spanish: "Español",
    english: "English",
    language: "Idioma",
    systemLanguage: "Como el sistema",
    title: "Ajustes",
    leads: {
      general: "Tu cuenta, este Mac y el aspecto de Orbit.",
      privacy: "Qué puede ver y oír Orbit en este Mac, y cómo pararlo todo.",
      recording: "Qué hace Orbit cuando empiezas una videollamada.",
      assistants:
        "Cada asistente se edita desde su conversación: el lápiz cambia su nombre, descripción y color; el engranaje abre el resto de sus ajustes.",
      meetings:
        "Cómo se escriben las notas de tus reuniones y qué palabras corrige la transcripción. Lo que pasa después a tu vault está en Ajustes › Notas.",
    },
    index: "Secciones de ajustes",
    missing: (count: number) =>
      pluralRules().select(count) === "one" ? "Falta 1" : `Faltan ${numberFormat().format(count)}`,
    appVersion: (version: string) => `Orbit ${version}`,
    exit: "Salir de Orbit",
    /** Closes Orbit in order and opens it again: the helper starts with it. */
    restart: "Reiniciar Orbit",

    // General
    account: "Tu cuenta",
    yourName: "Tu nombre",
    yourNameHelp: "Así te saluda Orbit cada mañana.",
    yourNamePlaceholder: "Cómo quieres que te salude Orbit",
    thisMac: "Este Mac",
    pairedWith: (host: string) => `Emparejado con ${host}`,
    unpaired: "Este Mac no está emparejado.",
    revoked: "El hub retiró este Mac. Vuelve a emparejarlo.",
    pair: "Emparejar",
    pairAgain: "Emparejar de nuevo",
    pairAgainQuestion: "¿Emparejar este Mac otra vez?",
    confirmPairAgain: "Sí, emparejar de nuevo",
    cancelPairAgain: "Cancelar",
    reopenOnboarding: "Volver a ver la bienvenida",
    appearance: "Aspecto",
    theme: "Tema",
    dark: "Oscuro",
    light: "Claro",
    startup: "Al encender el Mac",
    openAtLogin: "Abrir Orbit al iniciar sesión",
    // The launcher could not be read when the app started.
    loginUnknown: "macOS no dijo si Orbit se abre al iniciar sesión. Elígelo aquí.",
    updates: "Actualizaciones",
    version: (version: string) => `Versión ${version}`,
    // The app checks at start and every six hours
    // (`AUTOMATIC_CHECK_INTERVAL` in orbit-app-core's update.rs).
    updatesIdle: "Orbit busca versiones nuevas al abrirse y cada 6 horas",
    updatesDisabled:
      "Esta compilación no tiene un origen de actualizaciones configurado (ORBIT_UPDATE_BASE_URL).",
    searchUpdate: "Buscar actualización",
    checking: "Buscando una versión nueva…",
    available: (version: string | null) =>
      version === null ? "Hay una versión nueva" : `Hay una versión nueva: ${version}`,
    upToDate: "Orbit está al día",
    installing: "Instalando…",
    updateFailed: "No se pudo buscar la actualización. Vuelve a intentarlo.",
    lastInstallFailed: "La última actualización no terminó. Vuelve a intentarlo.",
    installRestart: "Instalar y reiniciar",
    updateNotice: (version: string) => `Hay una versión nueva de Orbit (${version})`,
    updateAction: "Actualizar",

    // Privacidad y permisos
    stopTitle: "Parar todo",
    stoppedTitle: "Todo está parado",
    stoppedText:
      "Orbit no graba, no escucha y no sube nada. Sigue así aunque lo cierres. Reactivar no vuelve a grabar por sí solo.",
    stopNow: "Parar todo ahora",
    rearm: "Reactivar Orbit",
    rearming: "Reactivando…",
    stopShortcut: `También con ${globalShortcuts.stop} desde cualquier app`,
    systemPermissions: "Permisos de macOS",
    granted: "Permitido",
    notGranted: "Sin permiso",
    checkingPermission: "Comprobando…",
    request: "Permitir",
    requestPermission: "Pedir permiso",
    openSystemSettings: "Abrir Ajustes del Sistema",
    helperMissing: "Orbit.app no está en marcha. Ábrela para ver los permisos.",
    microphoneReady: "Micrófono siempre listo",
    microphoneHint: `para notas de voz con ${globalShortcuts.ptt}`,
    microphoneBlocked: "Permítelo antes en Ajustes del Sistema",
    improve: "Ayudar a mejorar Orbit",
    errorReports: "Enviar informes de errores anónimos",
    errorReportsHelp:
      "Solo códigos de error. Nunca el contenido de tus reuniones ni de tus conversaciones.",

    // Recording and calls
    whenCall: "Cuando empieces una videollamada",
    whenCallLabel: "Al detectar una llamada",
    shortcuts: "Atajos desde cualquier app",

    // Assistants: a summary; each one's own settings are behind its gear.
    yourAssistants: "Tus asistentes",
    theirSettings: "Sus ajustes",
    theirSettingsLabel: (name: string) => `Ajustes de ${name}`,
    sessionExpired: "La sesión caducó",
    usage: (percent: number, window: string) =>
      `${numberFormat().format(percent)} % del uso ${window}`,
    primaryHelp:
      "Responde en el panel flotante, en «Preguntar» de las notas y cuando dices «Oye…» sin nombre. A los demás les llamas por su nombre.",
    primaryLabel: "Principal",
    primarySaving: "Guardando en el hub…",
    controlHistory: "Lo que han hecho en este Mac",
    historyUnreadable: "No se pudo leer lo que ha hecho en este Mac.",
    today: "Hoy",
    yesterday: "Ayer",

    // Meetings
    templates: "Plantillas",
    dictionaries: "Diccionarios",
  },
  // A permission row: what Orbit uses it for, as the person reads it.
  permissionRows: {
    microphone: { title: macPermissions.microphone.label },
    screen: { title: "Pantalla y audio del sistema" },
    accessibility: { title: macPermissions.accessibility.label },
    calendars: {
      title: macPermissions.calendar.label,
      help: "Para ver y cambiar tu agenda desde Orbit.",
    },
  },
  fullControl: {
    performed: "Hecho",
    cancelled: "Cancelado",
    failed: "Falló",
    unknown: "Resultado incierto",
  },
  ...settingsLists,
} as const;

export function settingsCopy(assistantName: string) {
  return {
    ...base,
    settings: {
      ...base.settings,
      deimos: assistantName,
      engine: {
        helperNotConnected: "Orbit.app no está en marcha. Ábrela para continuar.",
        missing: (names: string) =>
          `${assistantName} necesita ${names} para usar este Mac. Permítelo en Privacidad y permisos.`,
        failed: `${assistantName} no puede usar este Mac ahora. Orbit lo vuelve a intentar.`,
        screenRecording: macPermissions.screen_recording.name,
        and: " y ",
      },
      privacyArmed: `Corta al instante todo lo que Orbit escucha o mira: grabaciones, llamadas, notas de voz y «Oye ${assistantName}». Sigue parado aunque cierres Orbit.`,
      pairAgainWarning: `${assistantName} lo verá como un dispositivo nuevo y necesitarás un código nuevo.`,
      shortcutList: [
        [globalShortcuts.meeting, "Empezar o terminar una grabación"],
        [
          globalShortcuts.ptt,
          `Mantener para hablar con ${assistantName}; en una reunión, pregunta sobre ella`,
        ],
        [globalShortcuts.call, `Llamar a ${assistantName} o colgar`],
        [globalShortcuts.stop, "Parar todo al instante"],
      ],
    },
    permissionRows: {
      ...base.permissionRows,
      microphone: {
        ...base.permissionRows.microphone,
        help: `Para grabar reuniones, mandar notas de voz y hablar con ${assistantName}.`,
      },
      screen: {
        ...base.permissionRows.screen,
        help: `Para grabar el sonido de tus videollamadas y que ${assistantName} vea la pantalla si se lo pides. Sin él, Orbit solo graba tu micrófono.`,
      },
      accessibility: {
        ...base.permissionRows.accessibility,
        help: `Para que ${assistantName} pueda usar tu Mac y leer el texto que selecciones.`,
      },
    },
    fullControl: {
      ...base.fullControl,
      noHistory: `${assistantName} aún no ha hecho nada en este Mac.`,
    },
  } as const;
}
