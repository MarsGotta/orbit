import { macPermissions } from "./permissions";
import { globalShortcuts } from "./shortcuts";

const base = {
  settings: {
    // REF-11: what lets Deimos see and use this Mac is the macOS permission,
    // and nothing here switches it on. SIM-24: the card only speaks when
    // something is wrong, and then says what to do.
    engine: {
      helperNotConnected: "Orbit.app no está en marcha. Ábrela para continuar.",

      screenRecording: macPermissions.screen_recording.name,
      and: " y ",
    },
    microphoneReady: "Micrófono siempre listo",
    microphoneHelp: `Para notas de voz con ${globalShortcuts.ptt}`,
    // P9-05: Ajustes › Notas holds the note templates.
    notes: "Notas",
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
    restart: "Reiniciar Orbit",
    request: "Pedir",
    openSystemSettings: "Abrir Ajustes del sistema",
    startup: "Arranque",
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
    checking: "Buscando actualización…",
    available: (version: string | null) =>
      version === null ? "Hay una actualización" : `Hay una actualización: ${version}`,
    upToDate: "Orbit está al día.",
    installing: "Instalando…",
    updateFailed: "No se pudo actualizar Orbit. Vuelve a intentarlo.",
    lastInstallFailed: "La última actualización no terminó. Vuelve a intentarlo.",
    installRestart: "Instalar y reiniciar",
    updateNotice: (version: string) => `Hay una versión nueva de Orbit (${version})`,
    updateAction: "Actualizar",
    privacy: "Privacidad",
    privacyActive: "Todo está parado hasta que rearmes Orbit.",
    privacyArmed: "Detiene todo lo que Orbit escucha o mira hasta que lo rearmes.",
    errorReports: "Enviar informes de errores anónimos",
    errorReportsHelp:
      "Sólo códigos de error, nunca el contenido de tus reuniones ni de tus conversaciones.",
    stopNow: "Parar todo ahora",
    rearm: "Rearmar",
    exitSection: "Salir de Orbit",
    exit: "Salir de Orbit",
    reopenOnboarding: "Reabrir primer arranque",
  },
  fullControl: {
    performed: "Hecho",
    cancelled: "Cancelado",
    failed: "Falló",
    unknown: "Resultado incierto",
  },
  onboarding: {
    welcome: "Bienvenida a Orbit",
    welcomeBody: "Configura una vez el hub y los permisos del sistema.",
    continue: "Continuar",
    pair: "Emparejar",
    pairBody: "Conecta este Mac con tu hub de Orbit.",
    pairing: "Emparejando…",
    paired: "Este Mac ya está emparejado.",
    hubUrl: "URL del hub",
    code: "Código",
    deviceName: "Nombre del dispositivo",
    permissions: "Permisos",
    permissionsBody: "Puedes concederlos ahora o volver más tarde desde Ajustes.",
    accessibility: macPermissions.accessibility.label,
    ready: "Listo",
    readyBody: "Orbit queda preparado. Puedes cambiar cualquier opción desde Ajustes.",
    skipStep: "Saltar este paso",
    start: "Empezar a usar Orbit",
  },
  // P9-11: «Al detectar una llamada», a preference and not a permission.
  callSettings: {
    section: "Llamadas",
    preferenceTitle: "Al detectar una llamada",
    notify: "Avisarme",
    recordWithoutAsking: "Grabar sin preguntar",
    nothing: "No hacer nada",
  },
} as const;

export function settingsCopy(assistantName: string) {
  return {
    ...base,
    settings: {
      ...base.settings,
      deimos: assistantName,
      engine: {
        ...base.settings.engine,
        missing: (names: string) =>
          `${assistantName} necesita ${names} para usar este Mac. Concédelos en Permisos del sistema.`,
        failed: `${assistantName} no puede usar este Mac ahora. Orbit lo vuelve a intentar.`,
      },
      pairAgainWarning: `${assistantName} verá este Mac como un dispositivo nuevo.`,
    },
    fullControl: {
      ...base.fullControl,
      noHistory: `${assistantName} aún no ha hecho nada en este Mac.`,
    },
  } as const;
}
