// P9-04: the note screen. SIM-24: each step says where it stands and, when
// it failed, what to do; the hub's codes and numbers stay in the log.
const noteFailed = "No se pudo escribir la nota. Pulsa «Volver a escribir».";

const base = {
  noteScreen: {
    rewrite: "Volver a escribir",
    rewriteSetsAside: "Al volver a escribir, tus cambios se apartan y puedes recuperarlos.",
    whileRecording: "La nota se escribe al terminar la grabación.",
    template: "Plantilla",
    titleLabel: "Título de la reunión",
    titleNotOnHub: "Podrás renombrarla cuando la grabación termine de subirse al hub.",
    language: "Idioma",
    retranscribing: (done: number, total: number) => `Transcribiendo de nuevo… ${done} de ${total}`,
    templatesFailed: "No se pudieron cargar las plantillas.",
    templateChange: (name: string) => `Escribir la nota con «${name}»`,
    progressLabel: "Progreso de la nota",
    steps: { upload: "Subida", transcript: "Transcripción", summary: "Resumen" },
    stepStates: {
      active: "en curso",
      waiting: "esperando",
      done: "hecho",
      failed: "fallo",
    },
    causes: {
      recording: "Grabando",
      uploading: "Subiendo el audio…",
      hubUnavailable: "Sin conexión con el hub.",
      transcribing: "Transcribiendo…",
      transcriptFailed: "Parte de la transcripción falló. Pulsa «Reintentar».",
      afterRecording: "La nota se escribe al terminar la grabación.",
      notRequested: "Esta grabación no tiene nota. Pulsa «Volver a escribir».",
      waitingTranscript: "Esperando a la transcripción…",
      writing: "Escribiendo la nota…",
      retrying: "Reintentando…",
      failed: noteFailed,
    },
    supersededAvailable: "Al volver a escribir la nota se apartó tu versión anterior.",
    restoreSuperseded: "Recuperar mi versión",
    copySummary: "Copiar resumen",
    edit: "Editar",
    done: "Listo",
    editSection: (name: string) => `Editar «${name}»`,
    sectionLabel: (name: string) => `Texto de «${name}»`,
    copyActions: "Copiar todas",
    unassigned: "Sin asignar",
    you: (name: string) => `${name} (tú)`,
    actionDue: (due: string) => `Plazo: ${due}`,
    copyTranscript: "Copiar transcripción",
    searchLabel: "Buscar en la transcripción",
    searchPlaceholder: "Buscar…",
    searchCount: (current: number, total: number) => `${current} / ${total}`,
    searchNone: "Sin resultados",
    searchNext: "Siguiente",
    searchPrevious: "Anterior",
    searchClose: "Cerrar búsqueda",
    listRecording: "Grabando",
    listUploading: "Subiendo el audio…",
    listTranscribing: "Transcribiendo…",
    listTranscribed: "Transcripción lista",
    listTranscriptFailed: "No se pudo transcribir",
    listWriting: "Escribiendo la nota…",
    listFailed: "No se pudo escribir la nota",
  },
} as const;

export function noteScreenCopy(assistantName: string) {
  return {
    ...base,
    noteScreen: { ...base.noteScreen, askDeimos: `Preguntar a ${assistantName}` },
  } as const;
}
