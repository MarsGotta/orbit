// The note of a meeting. Each step says where it stands and, when it failed,
// what to do; the hub's codes and numbers stay in the log.
import { listOf } from "./notes";

const left = (value: number) =>
  value === 0 ? "Todo hecho" : value === 1 ? "Queda 1 tarea" : `Quedan ${value} tareas`;

const base = {
  noteScreen: {
    titleLabel: "Título de la reunión",
    titleNotOnHub: "Podrás renombrarla cuando la grabación termine de subirse al hub.",
    whileRecording: "La nota se escribe al terminar la grabación.",
    withPeople: (names: string[]) => listOf([...names, "tú"]),
    minutes: (value: number) => `${value} min`,
    hours: (hours: number, minutes: number) =>
      minutes === 0 ? `${hours} h` : `${hours} h ${minutes} min`,

    // How Orbit writes this note: proposals 8 and 9, nothing rewritten on a
    // mere change.
    howLabel: (template: string, language: string | null) =>
      language === null ? `Plantilla: ${template}` : `Plantilla: ${template} · ${language}`,
    howTitle: "Cómo escribe Orbit esta nota",
    template: "Plantilla",
    language: "Idioma de la reunión",
    dictionary: "Diccionario de palabras",
    dictionaryFromSeries: "El de la serie",
    noDictionary: "Ninguno",
    useForSeries: "Usar lo mismo en toda la serie",
    rewriteWarning: (language: string | null) =>
      `Orbit volverá a escribir la nota${language === null ? "" : ` y a transcribir la reunión en ${language}`}. Si cambiaste el resumen, tu versión queda guardada.`,
    rewriteNote: "Reescribir la nota",
    rewriting: (template: string) =>
      `Reescribiendo la nota con «${template}». Tu versión anterior queda guardada.`,
    retranscribing: (done: number, total: number) => `Transcribiendo de nuevo… ${done} de ${total}`,
    templatesUnread: "Las plantillas aún no se han leído del hub.",
    templatesFailed: "No se pudieron leer las plantillas del hub.",
    dictionariesUnread: "Los diccionarios aún no se han leído del hub.",
    seriesRefused: (code: string) => seriesCodes[code],
    close: "Cerrar",
    save: "Guardar",
    redoSummary: "Rehacer resumen",

    // Writing the note.
    preparing: "Orbit está preparando tu nota",
    preparingBody:
      "Suele tardar menos de un minuto. Puedes seguir con lo tuyo; te avisaremos cuando esté lista.",
    progressLabel: "Progreso",
    steps: {
      upload: {
        done: "Audio guardado",
        active: "Guardando el audio…",
        waiting: "Guardar el audio",
        failed: "No se pudo guardar el audio",
      },
      transcript: {
        done: "Transcripción lista",
        active: "Transcribiendo…",
        waiting: "Transcribir",
        failed: "No se pudo transcribir",
      },
      summary: {
        done: "Resumen escrito",
        active: "Escribiendo el resumen…",
        waiting: "Escribir el resumen",
        failed: "No se pudo escribir el resumen",
      },
    },
    hubUnavailable: "Sin conexión con el hub: sigue cuando vuelva.",
    noteFailed: "No se pudo escribir la nota",
    noteFailedBody: "El hub no pudo escribirla. Vuelve a intentarlo.",
    notConfigured:
      "El hub no tiene un modelo para escribir notas. Configúralo en el hub y vuelve a intentarlo.",
    notRequested: "Esta reunión aún no tiene nota. Pulsa «Rehacer resumen» para escribirla.",
    transcriptFailed: "No se pudo transcribir toda la reunión",
    transcriptFailedBody: "Parte del audio no se pudo transcribir. Vuelve a intentarlo.",
    retry: "Reintentar",

    // The parts of the note.
    partsLabel: "Partes de la nota",
    summaryTab: "Resumen",
    actionsTab: "Acciones",
    actionsTabCount: (value: number) => `Acciones · ${value}`,
    transcriptTab: "Transcripción",
    privateTab: "Notas privadas",

    copySummary: "Copiar resumen",
    summaryLater: (template: string) =>
      `El resumen aparecerá aquí en cuanto se escriba, con las secciones de tu plantilla «${template}».`,
    decisions: "Decisiones",
    openTopics: "Temas abiertos",
    whoSpoke: "Quién habló",
    share: (name: string, percent: number) => `${name} ${percent} %`,
    supersededAvailable: "Al reescribir la nota se apartó tu versión anterior.",
    restoreSuperseded: "Recuperar mi versión",
    edit: "Editar",
    done: "Listo",
    editSection: (name: string) => `Editar «${name}»`,
    sectionLabel: (name: string) => `Texto de «${name}»`,

    copyActions: "Copiar todas",
    actionsLeft: left,
    actionsLater:
      "Las tareas aparecerán cuando se escriba la nota, agrupadas por persona y con las tuyas primero.",
    noActions: "No salió ninguna tarea.",
    unassigned: "Sin asignar",
    you: "Tú",
    actionDue: (due: string) => `para el ${due}`,

    privateHelp: "Solo las ves tú. Orbit las usa para escribir la nota.",
    privateLabel: "Notas privadas",
    privatePlaceholder: "Escribe tus notas privadas…",

    // The notice when a note is ready (proposal 7).
    ready: "Tu nota está lista",
    open: "Abrir",

    // The list's one line.
    listRecording: "Grabando",
    listUploading: "Guardando el audio…",
    listTranscribing: "Transcribiendo…",
    listTranscribed: "Transcripción lista",
    listTranscriptFailed: "No se pudo transcribir",
    listWriting: "Escribiendo la nota…",
    listFailed: "No se pudo escribir la nota",
  },
} as const;

// «Usar lo mismo en toda la serie» refused by the hub, by its code; nothing
// was saved. Other codes fall back to the library's or the dictionary's words.
const seriesCodes: Record<string, string | undefined> = {
  meeting_series_missing: "Esta reunión no pertenece a una serie del calendario.",
  recording_not_found: "El hub ya no tiene esta reunión.",
  hub_route_missing: "Tu hub aún no sabe guardar esto para la serie: actualízalo.",
};

export function noteScreenCopy(assistantName: string) {
  return {
    ...base,
    noteScreen: { ...base.noteScreen, askDeimos: `Preguntar a ${assistantName}` },
  } as const;
}
