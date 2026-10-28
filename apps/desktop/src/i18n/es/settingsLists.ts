import { numberFormat } from "../locale";
import { getLocale } from "../locale";
/** Thousands with a dot, as Spanish writes them: 16.384. */
const number = (value: number) => value.toLocaleString(getLocale());

/** Ajustes' lists: the assistant's folders and what a detected call does. */
export const settingsLists = {
  // The assistant's folders: choosing one in the macOS panel is the whole
  // permission; nothing here switches it on or off.
  folders: {
    section: "Carpetas que pueden leer",
    help: "Las mismas para todos tus asistentes. Pueden leer estos archivos, nunca cambiarlos.",
    count: (count: number, max: number) =>
      `${numberFormat().format(count)} de ${numberFormat().format(max)}`,
    add: "Añadir carpeta",
    opening: "Abriendo…",
    remove: (name: string) => `Quitar ${name}`,
    removeLabel: "Quitar",
    empty: "Todavía no hay carpetas. Añade una para que los asistentes puedan leerla.",
    accessible: "Se puede leer",
    unknown: "Comprobando…",
    unreadable: "No se pudieron leer las carpetas guardadas.",
    full: (max: number) =>
      `${numberFormat().format(max)} carpetas como máximo. Quita una para añadir otra.`,
    notReady: "Orbit todavía está arrancando",
    readdHint: "Quítala y vuelve a añadirla.",
    missing: "No se encuentra: se borró o su disco no está conectado.",
    notAFolder: "Ya no es una carpeta.",
    accessDenied: "macOS no deja abrirla.",
    tooLong: (bytes: number, max: number) =>
      `Su ruta ocupa ${number(bytes)} bytes; Orbit admite ${number(max)}.`,
    bookmarkTooLarge: (bytes: number, max: number) =>
      `Su marcador ocupa ${number(bytes)} bytes; Orbit admite ${number(max)}.`,
    disk: (name: string) => `Disco «${name}»`,
    // The folders of the home folder, as Finder names them in Spanish.
    homeFolders: { Desktop: "Escritorio", Documents: "Documentos", Downloads: "Descargas" },
  },
  // «Al detectar una llamada», a preference and not a permission.
  callSettings: {
    options: {
      notify: {
        label: "Avisarme (recomendado)",
        help: "Orbit te pregunta si quieres grabarla.",
      },
      record: {
        label: "Grabar sin preguntar",
        help: "Empieza a grabar sola y te avisa de que lo hace.",
      },
      nothing: { label: "No hacer nada", help: "Grabas tú cuando quieras." },
    },
  },
} as const;
