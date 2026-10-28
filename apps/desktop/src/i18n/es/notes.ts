import { numberFormat } from "../locale";
import { transcriptionRefusals } from "./transcription";
import { mmss } from "../../time-format";

// Reuniones: the list of meetings and the transcript of one. Each meeting
// has a note; «Notas» names the vault that comes later (decided 2026-10-22).
const count = (value: number, one: string, many: string) =>
  `${numberFormat().format(value)} ${value === 1 ? one : many}`;

/** «Bruno», «Bruno y Celia», «Bruno, Celia y Dora». */
export function listOf(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} y ${items[items.length - 1]}`;
}

export const notes = {
  notes: {
    section: "Reuniones",
    recordedOn: (name: string) => `Grabada en ${name}`,
    pendingAudioOn: (name: string) =>
      `El audio pendiente debe subirse desde ${name}. Mantén Orbit abierto allí hasta que termine.`,
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
    // A meeting deleted or thrown away, here or on another Mac: not a
    // failure, and nothing to retry.
    noteGone: "Esta reunión ya no existe",
    noteGoneBody: "Se borró, en este Mac o en otro. Elige otra en la lista.",
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
    // Transcribing it again (annex, change 7): what it does first, then how
    // far it got, then what is left to do. The hub gives no estimate before
    // the pass starts, so the question does not promise how long it takes.
    retranscribeTitle: "¿Rehacer la transcripción?",
    retranscribeBody: (length: string, language: string | null, dictionaries: string[]) =>
      `Orbit vuelve a escuchar la grabación (${length})${language === null ? "" : ` en ${language}`}${dictionaries.length === 0 ? "" : ` con ${listOf(dictionaries)}`}. Puedes seguir con lo tuyo mientras tanto.`,
    retranscribeKeeps:
      "Se mantienen los nombres que pusiste a cada persona. El resumen no cambia hasta que lo rehagas.",
    // A hub up to 0.10.1 writes the note again after the pass (proposal 35's fallback).
    retranscribeKeepsOlderHub:
      "Se mantienen los nombres que pusiste a cada persona. Después, Orbit reescribe la nota.",
    stillRecording: "No se puede rehacer la transcripción: la grabación todavía no se ha cerrado.",
    finalPassQueued: "En cola para rehacer la transcripción.",
    finalPassRunning: "Rehaciendo la transcripción…",
    finalPassAside: " · puedes seguir con lo tuyo",
    remaining: (ms: number) => {
      const minutes = Math.ceil(ms / 60_000);
      return minutes <= 1 ? "menos de 1 min" : `unos ${numberFormat().format(minutes)} min`;
    },
    retranscribed: "Transcripción rehecha.",
    retranscribedRedo: "Transcripción rehecha. Rehaz el resumen para que la nota la use.",
    finalPassFailed: (code: string | null, timeoutMs?: number, providerStatus?: number) => {
      if (code && transcriptionRefusals[code])
        return `${transcriptionRefusals[code]}${providerStatus === undefined ? "" : ` (HTTP ${numberFormat().format(providerStatus)})`} Se conserva la transcripción anterior.`;
      if (code === "final_pass_incomplete")
        return "La transcripción nueva perdía partes de la anterior, así que se conserva la anterior.";
      if (code === "final_pass_timeout") {
        const budget =
          timeoutMs == null
            ? "lo que permite el hub"
            : `los ${numberFormat({ maximumFractionDigits: 1 }).format(timeoutMs / 60000)} minutos que permite el hub`;
        return `La transcripción nueva tardó más de ${budget}; se conserva la anterior.`;
      }
      return `No se pudo completar la nueva transcripción; se conserva la anterior.${code ? ` (${code})` : ""}`;
    },
    // A stretch the hub never got (a chunk it refused), said where it is.
    missingAudio: (track: string, atMs: number | null, durationMs: number | null) => {
      const source = track === "mic_local" ? "del micrófono" : "de la reunión";
      const from = atMs === null ? "" : ` desde ${mmss(atMs)}`;
      const length =
        durationMs === null
          ? ""
          : ` durante ${numberFormat().format(Math.max(1, Math.round(durationMs / 1_000)))} s`;
      return `Falta el audio ${source}${from}${length}: no llegó al hub y esa parte no está en la transcripción.`;
    },
    // «Silenciar»: a stretch where the microphone was muted, from the meeting's start.
    mutedMicrophone: (startMs: number, endMs: number) =>
      `Micrófono silenciado · ${mmss(startMs)} – ${mmss(endMs)}`,
    // A completed pass that kept spans of the live draft where the new text
    // lost them or looped: not a failure, the better of the two was used.
    keptDraft: (spans: number) =>
      `En ${numberFormat().format(spans)} ${spans === 1 ? "tramo" : "tramos"} se mantuvo la transcripción en vivo, que era mejor.`,
    unmatchedSpeakerNames: (value: number) =>
      `${numberFormat().format(value)} ${value === 1 ? "nombre de una voz que ya no está no se trasladó" : "nombres de voces que ya no están no se trasladaron"} a otras voces.`,
    // The harness reads the transcript from this line on (scripts/e2e-mac/checks.py).
    transcriptHelp:
      "Pulsa un nombre para cambiar quién habla. Pasa el ratón por una palabra y púlsala para corregirla: la corrección se guarda en los diccionarios de esta reunión.",
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
    } as const,
    // The same languages inside a sentence: «en español con Nubaria».
    languagesInSentence: {
      es: "español",
      en: "inglés",
      ca: "catalán",
      fr: "francés",
      pt: "portugués",
      de: "alemán",
      it: "italiano",
    } as const,

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
      turns === 1
        ? `En todos los turnos de ${name}`
        : `En los ${numberFormat().format(turns)} turnos de ${name}`,
    back: "Atrás",
    applySpeaker: "Aplicar",
    renamed: (turns: number) => `Cambiado en ${count(turns, "turno", "turnos")}.`,
    usesOnRewrite: "La nota usará el cambio cuando rehagas el resumen.",

    // Correcting a word (annex, change 5): the menu beside it, then the band.
    pressToCorrect: "Pulsa para corregir",
    correct: (word: string) => `Corregir «${word}»`,
    timesHere: (times: number) =>
      times === 1
        ? "Sale 1 vez en esta reunión."
        : `Sale ${numberFormat().format(times)} veces en esta reunión.`,
    shouldSay: "Debe decir",
    // A dictionary corrects every place it is said: the hub has no
    // correction for one place alone, so this cannot be unticked.
    correctEvery: (times: number) => `Corregir las ${numberFormat().format(times)} veces`,
    addTo: "Se añade a · puedes elegir varios",
    chooseDictionaries: "Elige uno o varios",
    thisMeeting: "esta reunión",
    dictionaryDetail: (series: string[], words: number) =>
      `${series.length === 0 ? "Ninguna serie" : listOf(series)} · ${count(words, "palabra", "palabras")}`,
    newDictionary: "+ Crear un diccionario nuevo",
    dictionaryName: "Nombre del diccionario",
    appliesIn: (names: string[]) =>
      `Orbit la corregirá en las reuniones que usen ${listOf(names)}.`,
    chooseOne: "Elige al menos un diccionario para guardar la corrección.",
    // A selection that is not whole words: a dictionary would never find it.
    notWhole: (text: string) =>
      `«${text}» no sale entera en esta reunión: un diccionario corrige palabras o frases enteras.`,
    correctAndAdd: "Corregir y añadir",
    corrected: (wrong: string, right: string, places: number, names: string[]) =>
      `«${wrong}» ahora es «${right}» en ${count(places, "sitio", "sitios")} y está en ${listOf(names)}.`,
    undo: "Deshacer",
    undone: (wrong: string, names: string[]) =>
      `Corrección deshecha. «${wrong}» ya no está en ${listOf(names)}.`,
    // Not drawn: two dictionaries of the meeting hear the form otherwise.
    conflict: (other: string, wrong: string, otherCorrect: string, applied: string | null) =>
      `${other} corrige «${wrong}» por «${otherCorrect}». Cuando dos diccionarios no coinciden, gana el primero${applied === null ? "" : `: en esta reunión, ${applied}`}.`,

    save: "Guardar",
    cancel: "Cancelar",
  },
} as const;
