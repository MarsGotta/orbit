import { macPermissions } from "./permissions";
import { globalShortcuts } from "./shortcuts";

const base = {
  settings: {
    // What lets Deimos see and use this Mac is the macOS permission,
    // and nothing here switches it on. The card only speaks when
    // something is wrong, and then says what to do.
    engine: {
      helperNotConnected: "Orbit.app no está en marcha. Ábrela para continuar.",

      screenRecording: macPermissions.screen_recording.name,
      and: " y ",
    },
    microphoneReady: "Micrófono siempre listo",
    microphoneHelp: `Para notas de voz con ${globalShortcuts.ptt}`,
    // Ajustes › Reuniones holds the note templates and dictionaries.
    notes: "Reuniones",
    account: "Cuenta",
    yourName: "Tu nombre",
    yourNamePlaceholder: "Cómo quieres que te salude Orbit",
    unpaired: "Este Mac no está emparejado.",
    revoked: "El hub retiró este Mac. Vuelve a emparejarlo.",
    pairAgain: "Emparejar de nuevo",

    confirmPairAgain: "Confirmar nuevo emparejamiento",
    cancelPairAgain: "Cancelar",
    systemPermissions: "Permisos del sistema",
    helperMissing: "Orbit.app no está en marcha. Ábrela para ver los permisos.",
    request: "Permitir",
    openSystemSettings: "Abrir Ajustes del Sistema",
    startup: "Al encender el Mac",
    openAtLogin: "Abrir Orbit al iniciar sesión",
    appearance: "Aspecto",
    theme: "Tema",
    dark: "Oscuro",
    light: "Claro",
    updates: "Actualizaciones",
    version: (version: string) => `Versión ${version}`,
    updatesDisabled:
      "Esta compilación no tiene un origen de actualizaciones configurado (ORBIT_UPDATE_BASE_URL).",
    searchUpdate: "Buscar actualización",
    checking: "Buscando una versión nueva…",
    available: (version: string | null) =>
      version === null ? "Hay una actualización" : `Hay una actualización: ${version}`,
    upToDate: "Orbit está al día",
    installing: "Instalando…",
    updateFailed: "No se pudo actualizar Orbit. Vuelve a intentarlo.",
    lastInstallFailed: "La última actualización no terminó. Vuelve a intentarlo.",
    installRestart: "Instalar y reiniciar",
    updateNotice: (version: string) => `Hay una versión nueva de Orbit (${version})`,
    updateAction: "Actualizar",
    privacy: "Privacidad",
    privacyActive:
      "Todo está parado. Orbit no graba, no escucha y no sube nada. Sigue así aunque lo cierres.",
    errorReports: "Enviar informes de errores anónimos",
    errorReportsHelp:
      "Solo códigos de error. Nunca el contenido de tus reuniones ni de tus conversaciones.",
    stopNow: "Parar todo ahora",
    rearm: "Reactivar Orbit",
    exitSection: "Salir de Orbit",
    exit: "Salir de Orbit",
    reopenOnboarding: "Volver a ver la bienvenida",
  },
  fullControl: {
    performed: "Hecho",
    cancelled: "Cancelado",
    failed: "Falló",
    unknown: "Resultado incierto",
  },
  onboarding: {
    welcome: "Te damos la bienvenida a Orbit",
    welcomeBody:
      "En un par de minutos lo dejas listo: conectas este Mac con tu hub y eliges qué puede usar Orbit.",
    continue: "Continuar",
    pair: "Emparejar",
    pairTitle: "Conecta este Mac con tu hub",
    pairBody:
      "El hub es donde se guardan tus reuniones y notas. Quien lo gestiona te da la dirección y un código.",
    pairing: "Emparejando…",
    paired: "Este Mac ya está emparejado.",
    hubUrl: "Dirección del hub",
    code: "Código de emparejamiento",
    deviceName: "Nombre de este Mac",
    permissions: "Elige qué puede usar Orbit",
    permissionsBody: "Puedes concederlos ahora o volver más tarde desde Ajustes.",
    accessibility: macPermissions.accessibility.label,
    ready: "Todo listo",
    readyBody: "Orbit queda preparado. Puedes cambiar cualquier opción desde Ajustes.",
    // The welcome skips the whole first run; a later step, only itself.
    skipAll: "Configurarlo más tarde",
    skipStep: "Hacerlo después",
    start: "Empezar a usar Orbit",
  },
  // The assistant's folders: choosing one in the macOS panel is the whole
  // permission; nothing here switches it on or off.
  folders: {
    section: "Carpetas que puede leer",
    add: "Añadir carpeta",
    remove: (path: string) => `Quitar ${path}`,
    removeLabel: "Quitar",
    empty: "Todavía no hay carpetas.",
    accessible: "Se puede leer",
    unknown: "Sin comprobar todavía",
    unreadable: "No se pudieron leer las carpetas guardadas.",
    full: (max: number) => `${max} carpetas como máximo. Quita una para añadir otra.`,
    missing: "No se encuentra: se borró o su disco no está conectado.",
    notAFolder: "Ya no es una carpeta.",
    accessDenied: "macOS no deja abrirla.",
    tooLong: (bytes: number, max: number) => `Su ruta ocupa ${bytes} bytes; Orbit admite ${max}.`,
    bookmarkTooLarge: (bytes: number, max: number) =>
      `Su marcador ocupa ${bytes} bytes; Orbit admite ${max}.`,
  },
  // «Cuando empieces una videollamada», a preference and not a permission.
  callSettings: {
    section: "Llamadas",
    preferenceTitle: "Cuando empieces una videollamada",
    notify: "Avisarme (recomendado)",
    recordWithoutAsking: "Grabar sin preguntar",
    nothing: "No hacer nada",
  },
} as const;

export function settingsCopy(assistantName: string) {
  return {
    ...base,
    settings: {
      ...base.settings,
      controlHistory: "Lo que ha hecho en este Mac",
      privacyArmed: `Corta al instante todo lo que Orbit escucha o mira: grabaciones, llamadas, notas de voz y «Oye ${assistantName}». Sigue parado aunque cierres Orbit.`,
      engine: {
        ...base.settings.engine,
        missing: (names: string) =>
          `${assistantName} necesita ${names} para usar este Mac. Concédelos en Permisos del sistema.`,
        failed: `${assistantName} no puede usar este Mac ahora. Orbit lo vuelve a intentar.`,
      },
      pairAgainWarning: `${assistantName} verá este Mac como un dispositivo nuevo.`,
      foldersHelp: `${assistantName} puede leer los archivos de estas carpetas, sin cambiarlos. Elegirlas en el panel de macOS es todo el permiso.`,
    },
    fullControl: {
      ...base.fullControl,
      noHistory: `${assistantName} aún no ha hecho nada en este Mac.`,
    },
  } as const;
}
