const base = {
  appName: "Orbit",
  loading: "Conectando con Orbit…",
  connectFailed: "No se pudo conectar con Orbit. Ciérralo y vuelve a abrirlo.",
  retry: "Reintentar",
  shell: {
    viewsLabel: "Vistas",
    breadcrumbsLabel: "Migas",
    contextLabel: "Panel de contexto",
    changeTheme: "Cambiar tema",
    themeTitle: "Tema",
    localUser: "Usuario local",
    loadingView: "Cargando vista…",
    notice: "Aviso de Orbit",
    closeNotice: "Cerrar aviso",
    briefing: "Hoy",
    calendar: "Calendario",
    // Each entry is a meeting; its note is one part of it.
    notes: "Reuniones",
    assistant: "Asistente",
    settings: "Ajustes",
  },
  design: {
    askOrbit: "Paleta de órdenes",
    commandInputLabel: "Comando",
    commandPlaceholder: "Pregunta, dicta o llama…",
    escape: "esc",
    undo: "Deshacer",
    expandCall: "Abrir",
    collapseCall: "Minimizar",
    hangUp: "Colgar",
    endCallFromOrb: "Colgar desde el orbe",
    callSubtitle: "Habla en voz alta",
  },
  context: {
    now: "ahora",
  },
  // The status pill only speaks when something is wrong, and then
  // says what to do. Nothing here names a server, a device or a count.
  problem: {
    orbitLost: "Orbit no responde. Ciérralo y vuelve a abrirlo.",
    privacyStopped: "Todo está parado",
    unpaired: "Este Mac no está emparejado",
    revoked: "El hub retiró este Mac",
    offline: "Sin conexión. Comprueba tu red.",
    missingPermission: (name: string) => `Falta el permiso de ${name}`,
    missingPermissions: "Faltan permisos",
    pair: "Emparejar",
    openSettings: "Abrir Ajustes",
    rearm: "Reactivar",
  },
} as const;

/** The window's frame, with the conversation's agent where it is asked or called. */
export function statusCopy(assistantName: string) {
  return {
    ...base,
    shell: { ...base.shell, ask: `Preguntar a ${assistantName}` },
    design: {
      ...base.design,
      inCall: `En llamada con ${assistantName}`,
      callAssistant: `Llamar a ${assistantName}`,
    },
  } as const;
}
