import { numberFormat } from "../locale";
export const dictionary = {
  dictionary: {
    label: "Diccionario",
    name: "Nombre del diccionario",
    inherit: "Usar el de la serie",
    create: "Crear un diccionario",
    correctText: "Corregir texto",
    correctForm: "Forma correcta",
    saveCorrection: "Guardar corrección",
    saving: "Guardando en el hub…",
    unread: "El diccionario aún no se ha podido leer del hub.",
    selected: (text: string) => `Texto seleccionado: «${text}»`,
    help: "Corrige todas las apariciones de esta forma en las reuniones que usan el diccionario. La nota ya escrita cambiará cuando rehagas el resumen.",
    empty: "Todavía no hay diccionarios. Crea uno al corregir una palabra de una transcripción.",
    emptyTerms: "Aún no corrige ninguna palabra.",
    noSeries: "Ninguna serie lo usa siempre",
    series: (names: string) => `Lo usan siempre: ${names}`,
    insteadOf: (wrong: string) => `en lugar de ${wrong}`,
    rename: "Cambiar nombre",
    renameLabel: (name: string) => `Cambiar el nombre de ${name}`,
    // Above the cards: several per meeting, and how words get in.
    several:
      "Una reunión o una serie puede usar varios diccionarios a la vez. Las palabras se añaden al corregirlas en una transcripción: pulsa una palabra y elige «Corregir».",
    words: (count: number) =>
      count === 1 ? "1 palabra" : `${numberFormat().format(count)} palabras`,
    seeAll: (count: number) => `Ver las ${numberFormat().format(count)} palabras`,
    removeEntry: "Quitar",
    remove: (term: string) => `Quitar ${term}`,
    refused: (code: string, detail: string, numbers: Record<string, number>) => {
      const causes: Record<string, string> = {
        not_paired: "Orbit no está emparejado con el hub",
        privacy_stopped: "Todo está parado",
        // The transport's own cause stays in the log.
        hub_unreachable: "No se guardó el cambio: no hay conexión con el hub",

        dictionary_full: "El diccionario está lleno",
        dictionary_text_too_long: "El término o nombre es demasiado largo",
        dictionary_forms_full: "El término tiene demasiadas formas equivocadas",
        dictionaries_full: "Hay demasiados diccionarios",
        dictionary_too_large: "El diccionario ocupa demasiados bytes",
        dictionaries_too_large: "La lista de diccionarios ocupa demasiados bytes",
        dictionary_series_full: "Hay demasiadas series asignadas",
        dictionary_series_missing: "La reunión no tiene serie de calendario",
        dictionary_not_found: "El diccionario no existe",
        dictionary_text_invalid: "El término o nombre está vacío o contiene controles",
      };
      const labels: Record<string, string> = {
        terms: "términos",
        max_terms: "máximo de términos",
        forms: "formas",
        max_forms: "máximo de formas",
        characters: "caracteres",
        max_characters: "máximo de caracteres",
        dictionaries: "diccionarios",
        max_dictionaries: "máximo de diccionarios",
        series: "series",
        max_series: "máximo de series",
        bytes: "bytes",
        max_bytes: "máximo de bytes",
        used_bytes: "bytes ocupados",
      };
      const values = Object.entries(numbers)
        .map(([key, value]) => `${labels[key] ?? key}: ${numberFormat().format(value)}`)
        .join(", ");
      return `${causes[code] ?? detail}${values ? ` (${values})` : ""}.`;
    },
  },
} as const;
