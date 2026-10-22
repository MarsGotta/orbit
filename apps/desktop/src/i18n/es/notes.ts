// Reuniones: the list of meetings and the transcript of one. Each meeting
// has a note; «Notas» names the vault that comes later (decided 2026-10-22).
const count = (value: number, one: string, many: string) => `${value} ${value === 1 ? one : many}`;

/** «Bruno», «Bruno y Celia», «Bruno, Celia y Dora». */
export function listOf(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} y ${items[items.length - 1]}`;
}

export const notes = {
  notes: {
    section: "Reuniones",
    listLabel: "Lista de reuniones",
    refresh: "Actualizar",
    loadMore: "Cargar más reuniones",
    loadingMore: "Cargando…",
    loadingIndex: "Leyendo tus reuniones del hub…",
    loadingNote: "Leyendo la nota del hub…",
    empty: "Aún no hay reuniones",
    emptyBody: "Graba tu primera reunión y Orbit escribirá la nota por ti.",
    record: "Grabar reunión",
    choose: "Elige una reunión",
    chooseBody: "Su nota, sus acciones y su transcripción se abren aquí.",
    // A read that failed says so and offers «Reintentar»; its cause
    // stays in the log.
    noteReadFailed: "No se pudo abrir la nota",
    noteReadFailedBody: "Vuelve a intentarlo. Tus reuniones siguen guardadas en el hub.",
    indexFailed: "No se pudo leer la lista de reuniones. Vuelve a intentarlo.",
    // The hub out of reach is not a failure; the connection says
    // itself at the top of the window.
    needsHub: "Tus reuniones se leen del hub. Aparecerán cuando Orbit se conecte.",
    retry: "Reintentar",
    today: "Hoy",
    yesterday: "Ayer",
    thisWeek: "Esta semana",
    last30Days: "Últimos 30 días",
    before: "Antes",

    // The transcript.
    retranscribe: "Rehacer transcripción",
    finalPassQueued: "En cola para rehacer la transcripción.",
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
    unmatchedSpeakerNames: (value: number) =>
      `${value} ${value === 1 ? "nombre de una voz que ya no está no se trasladó" : "nombres de voces que ya no están no se trasladaron"} a otras voces.`,
    transcriptHelp:
      "Pulsa un nombre para cambiar quién habla. Selecciona una palabra para corregirla.",
    transcriptArriving:
      "La transcripción va apareciendo. Pulsa un nombre para cambiar quién habla.",
    moreTranscript: "Llegando más transcripción",
    noTranscript: "La transcripción aún no está lista",
    transcriptFailed: "No se pudo cargar la transcripción.",
    searchLabel: "Buscar en la transcripción",
    searchResults: (value: number) =>
      value === 0 ? "Sin resultados" : count(value, "resultado", "resultados"),
    copy: "Copiar",
    copied: "✓ Copiado",
    languages: {
      es: "Español",
      en: "English",
      ca: "Català",
      fr: "Français",
      pt: "Português",
      de: "Deutsch",
      it: "Italiano",
    } as Record<string, string>,

    // Who speaks.
    nameIt: "· ponle nombre",
    whoIs: (name: string) => `¿Quién es ${name}?`,
    speakerSearch: "Buscar o añadir un nombre",
    guests: "Invitados de la reunión",
    recordingSpeakers: "Ya en esta grabación",
    addSomeone: "+ Añadir a alguien",
    addNamed: (name: string) => `+ Añadir «${name}»`,
    wherePut: (name: string) => `¿Dónde pongo «${name}»?`,
    onlyThisTurn: (at: string) => `Solo en este turno (${at})`,
    allSpeakerTurns: (turns: number, name: string) =>
      turns === 1 ? `En todos los turnos de ${name}` : `En los ${turns} turnos de ${name}`,
    back: "Atrás",
    applySpeaker: "Aplicar",
    renamed: (turns: number) => `Cambiado en ${count(turns, "turno", "turnos")}.`,
    corrected: (wrong: string, right: string) => `Corregido «${wrong}» por «${right}».`,
    usesOnRewrite: "La nota usará el cambio cuando rehagas el resumen.",

    // Correcting a word.
    correct: (word: string) => `Corregir «${word}»`,
    correctTitle: "Corregir una palabra",
    correctSubtitle: "Orbit la corregirá en todas las reuniones que usan este diccionario.",
    heard: "Se oyó",
    shouldSay: "Debe decir",
    dictionary: "Diccionario",
    dictionaryUsedBy: (name: string, series: string[]) =>
      series.length === 0 ? name : `${name} · lo usan ${listOf(series)}`,
    newDictionary: "Crear un diccionario nuevo",
    dictionaryName: "Nombre del diccionario",
    correctLater: "La nota ya escrita cambiará cuando rehagas el resumen.",
    saveCorrection: "Guardar corrección",
    savingCorrection: "Guardando en el hub…",

    save: "Guardar",
    cancel: "Cancelar",
  },
} as const;
