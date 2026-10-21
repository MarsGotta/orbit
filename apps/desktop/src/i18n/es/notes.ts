export const notes = {
  notes: {
    retranscribe: "Volver a transcribir",
    finalPassQueued: "En cola para volver a transcribir.",
    finalPassRunning: "Transcribiendo de nuevo…",
    finalPassFailed: (code: string | null, timeoutMs?: number) => {
      if (code === "final_pass_incomplete")
        return "La transcripción nueva perdía partes de la anterior, así que se conserva la anterior.";
      if (code === "final_pass_timeout") {
        const budget =
          timeoutMs == null
            ? "lo que permite el hub"
            : `los ${new Intl.NumberFormat("es", { maximumFractionDigits: 1 }).format(timeoutMs / 60000)} minutos que permite el hub`;
        return `La transcripción nueva tardó más de ${budget}; se conserva la anterior.`;
      }
      return `No se pudo completar la nueva transcripción; se conserva la anterior.${code ? ` (${code})` : ""}`;
    },
    unmatchedSpeakerNames: (count: number) =>
      `${count} ${count === 1 ? "nombre de una voz que ya no está no se trasladó" : "nombres de voces que ya no están no se trasladaron"} a otras voces.`,

    refresh: "Actualizar",
    refreshing: "Actualizando notas…",
    loadMore: "Cargar más notas",
    empty: "Aún no hay notas",
    emptyBody: "Graba una reunión para empezar.",
    loadingIndex: "Cargando notas…",
    loadingNote: "Cargando nota…",
    // A read that failed says so and offers «Reintentar»; its cause
    // stays in the log.
    noteReadFailed: "No se pudo abrir la nota.",
    indexFailed: "No se pudieron cargar las notas.",
    // The hub out of reach is not a failure; the connection
    // says itself in the status.
    needsHub: "Las notas se leen del hub. Aparecerán cuando Orbit se conecte.",
    today: "Hoy",
    yesterday: "Ayer",
    thisWeek: "Esta semana",
    last30Days: "Últimos 30 días",
    before: "Antes",
    privateTab: "Notas privadas",
    summaryTab: "Resumen",
    actionsTab: "Acciones",
    transcriptTab: "Transcripción",
    // The daemon keeps them encrypted on this Mac and sends them with
    // the note request, so the note is written with them.
    privateHelp: "Solo las ves tú. Se usan para escribir la nota.",
    privateLabel: "Notas privadas",
    privatePlaceholder: "Escribe tus notas privadas…",
    voiceShare: "Reparto de voz",
    summaryLater: "El resumen aparecerá cuando se escriba la nota.",
    actionsLater: "Las tareas aparecerán cuando se escriba la nota.",
    noActions: "No salió ninguna tarea",
    decisions: "Decisiones",
    openTopics: "Temas abiertos",
    noTranscript: "La transcripción aún no está lista",
    transcriptFailed: "No se pudo cargar la transcripción.",
    // The language the transcript is in, chosen (Spanish by
    // default), not detected; each one is named in its own language.
    // Distinct from the header's picker («Idioma»): the real test
    // reads this line (scripts/e2e-mac/comprobaciones.py).
    transcriptLanguage: "Idioma de la transcripción",
    languages: {
      es: "Español",
      en: "English",
      ca: "Català",
      fr: "Français",
      pt: "Português",
      de: "Deutsch",
      it: "Italiano",
    } as Record<string, string>,
    changeSpeaker: "Cambiar hablante",
    speakerSearch: "Buscar o añadir un nombre",
    recordingSpeakers: "En esta grabación",
    guests: "Invitados",
    addSpeaker: "Añadir a alguien",
    assignSpeaker: (name: string) => `Asignar «${name}»`,
    speakerScope: "¿Cómo quieres aplicar este cambio?",
    onlyThisTurn: "Sólo este turno",
    allSpeakerTurns: (name: string) => `Todos los turnos de ${name}`,
    applySpeaker: "Aplicar",
    speakerNoteHelp:
      "Una nota ya escrita conserva los nombres anteriores hasta que se vuelva a escribir.",
    speakerName: "Nombre del hablante",
    save: "Guardar",
    cancel: "Cancelar",
    retry: "Reintentar",
    transcribing: "Transcribiendo…",
    failedGroups: "Parte de la transcripción falló.",
  },
} as const;
