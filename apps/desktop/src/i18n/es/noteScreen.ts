// The note screen. Each step says where it stands and, when
// it failed, what to do; the hub's codes and numbers stay in the log.
const noteFailed = "No se pudo escribir la nota. Pulsa «Rehacer resumen».";

const base = {
  noteScreen: {
    rewrite: "Rehacer resumen",
    rewriteSetsAside: "Al rehacer el resumen, tus cambios se apartan y puedes recuperarlos.",
    whileRecording: "La nota se escribe al terminar la grabación.",
    template: "Plantilla",
    titleLabel: "Título de la reunión",
    titleNotOnHub: "Podrás renombrarla cuando la grabación termine de subirse al hub.",
    language: "Idioma de la reunión",
    retranscribing: (done: number, total: number) => `Transcribiendo de nuevo… ${done} de ${total}`,
    templatesFailed: "No se pudieron cargar las plantillas.",
    templateChange: (name: string) => `Escribir la nota con «${name}»`,
    progressLabel: "Progreso de la nota",
    partsLabel: "Partes de la nota",
    // Each step by where it stands: «Audio guardado → Transcribiendo… → Escribir el resumen».
    steps: {
      upload: {
        waiting: "Guardar el audio",
        active: "Guardando el audio…",
        done: "Audio guardado",
        failed: "No se pudo guardar el audio",
      },
      transcript: {
        waiting: "Transcribir",
        active: "Transcribiendo…",
        done: "Transcripción lista",
        failed: "No se pudo transcribir",
      },
      summary: {
        waiting: "Escribir el resumen",
        active: "Escribiendo el resumen…",
        done: "Resumen escrito",
        failed: "No se pudo escribir el resumen",
      },
    },
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
      notRequested: "Esta grabación no tiene nota. Pulsa «Rehacer resumen».",
      waitingTranscript: "Esperando a la transcripción…",
      writing: "Escribiendo la nota…",
      retrying: "Reintentando…",
      failed: noteFailed,
    },
    supersededAvailable: "Al rehacer el resumen se apartó tu versión anterior.",
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
