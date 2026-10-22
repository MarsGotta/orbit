import type { Refusal } from "../../bridge/refusal";
import { fields } from "./refusalDetails";

const KIB = 1024;
const MIB = 1024 * KIB;
const decimals = new Intl.NumberFormat("es-ES", { maximumFractionDigits: 1 });

/**
 * A size as the person reads it: «512 B», «1,5 KB», «20 MB». In units of
 * 1024, as the hub's ceilings are set, so a ceiling reads as a round number.
 */
export function fileSize(bytes: number): string {
  if (bytes < KIB) return `${bytes} B`;
  if (bytes < MIB) return `${decimals.format(bytes / KIB)} KB`;
  return `${decimals.format(bytes / MIB)} MB`;
}

function size(value: unknown): string {
  return typeof value === "number" ? fileSize(value) : "un tamaño desconocido";
}

/** The file between Spanish quotes «», or «el fichero» when the refusal has no name. */
function quoted(name: unknown): string {
  return typeof name === "string" && name !== "" ? `«${name}»` : "el fichero";
}

/** «de «plano.pdf»», or «del fichero»: Spanish contracts «de el». */
function ofFile(name: unknown): string {
  const file = quoted(name);
  return file === "el fichero" ? "del fichero" : `de ${file}`;
}

function capitalised(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** The conversation's ceiling: what it already holds, what the message adds, the maximum. */
export function sessionCeilingText(stored: unknown, bytes: unknown, maxBytes: unknown): string {
  const held =
    typeof stored === "number"
      ? `Esta conversación ya guarda ${size(stored)} de adjuntos y este mensaje añade ${size(bytes)}`
      : `Con ${size(bytes)} más de adjuntos, esta conversación se pasa`;
  return `${held}; el máximo por conversación es ${size(maxBytes)}. Abre una conversación nueva para adjuntar más.`;
}

export function tooManyText(files: unknown, maxFiles: unknown): string {
  return `Este mensaje llevaría ${String(files)} adjuntos y el máximo por mensaje es ${String(maxFiles)}.`;
}

export const ATTACHMENT_NOT_FOUND =
  "Ese adjunto ya no está en el hub (se borra si no se envía en una hora); vuelve a adjuntarlo.";

function tooLarge(cause: Record<string, unknown>): string {
  const max = size(cause.max_bytes);
  if (cause.ceiling === "file")
    return `${capitalised(quoted(cause.name))} ocupa ${size(cause.bytes)} y el máximo por fichero es ${max}.`;
  if (cause.ceiling === "session")
    return sessionCeilingText(cause.session_bytes, cause.bytes, cause.max_bytes);
  const files = typeof cause.files === "number" ? `Los ${cause.files} adjuntos` : "Los adjuntos";
  return `${files} de este mensaje ocupan ${size(cause.bytes)} y el máximo por mensaje es ${max}.`;
}

function unreadable(cause: Record<string, unknown>): string {
  const file = quoted(cause.name);
  const reasons: Record<string, string> = {
    missing: `${capitalised(file)} ya no está donde estaba.`,
    permission: `Orbit no tiene permiso para leer ${file}.`,
    not_a_file: `${capitalised(file)} no es un fichero: las carpetas no se adjuntan.`,
    io: `macOS no pudo leer ${file}.`,
    invalid_bytes: `El contenido ${ofFile(cause.name)} está dañado o no corresponde a su tipo.`,
    pdf_locked: `${capitalised(file)} está protegido con contraseña.`,
    pdf_damaged: `${capitalised(file)} está dañado: el hub no pudo leer el PDF.`,
    render_failed: `El hub no pudo convertir en imágenes las páginas ${ofFile(cause.name)}.`,
    // A HEIC photo, which the Mac turns into JPEG before uploading it.
    photo_unreadable: `macOS no pudo leer la foto ${file} para pasarla a JPEG.`,
    photo_jpeg_failed: `macOS no pudo pasar ${file} a JPEG.`,
    helper_restarted: `Orbit se interrumpió mientras pasaba ${file} a JPEG. Vuelve a adjuntarla.`,
  };
  return (
    reasons[String(cause.reason)] ?? `No se pudo leer ${file} (causa ${String(cause.reason)}).`
  );
}

function refusedByAgent(cause: Record<string, unknown>): string {
  const name = typeof cause.name === "string" ? ` «${cause.name}»` : "";
  const code = typeof cause.upstream_code === "number" ? ` (código ${cause.upstream_code})` : "";
  return `El agente no aceptó la imagen${name}${code}.`;
}

/** An attachment refusal in words: the file, and the numbers of the rule that fired. */
export function attachmentRefusalText(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  switch (cause.kind) {
    case "unsupported":
      return "Este hub no admite adjuntos; actualízalo.";
    case "too_large":
      return tooLarge(cause);
    case "too_many":
      return tooManyText(cause.files, cause.max_files);
    case "type_unsupported":
      return `${capitalised(quoted(cause.name))} es ${String(cause.mime)}: ningún agente lo lee. Se pueden adjuntar imágenes PNG, JPEG, GIF o WebP, PDF y ficheros de texto.`;
    case "unreadable":
      return unreadable(cause);
    case "refused":
      return refusedByAgent(cause);
    case "not_found":
      return ATTACHMENT_NOT_FOUND;
    default:
      return `No se pudo adjuntar por una causa que esta versión no reconoce (${String(cause.kind)}).`;
  }
}

/** Why «Adjuntar ventana» sent the capture without the window's text. */
const WINDOW_TEXT_MISSING: Record<string, string> = {
  empty: "La ventana no tiene texto legible; va solo la captura.",
  accessibility_permission_required:
    "Sin el permiso de Accesibilidad, Orbit no lee su texto; va solo la captura.",
  timeout: "Su app no dio el texto a tiempo; va solo la captura.",
};

/** What the composer and the messages say about attachments. */
export const attachmentsCopy = {
  attachments: {
    attach: "Adjuntar",
    attachWindow: "Adjuntar ventana",
    unsupported: "Este hub no admite adjuntos",
    window: "Ventana",
    remove: (name: string) => `Quitar ${name}`,
    uploading: "Subiendo…",
    pastedImage: "Imagen pegada.png",
    pastedText: "Texto pegado.txt",
    failed: (name: string, code: string) => `No se pudo adjuntar «${name}» (${code}).`,
    // Uploading, «Ver» or «Abrir» with the hub stopped or out of reach.
    hubUnreachable: (name: string) =>
      `Sin conexión con el hub: inténtalo con «${name}» cuando vuelva.`,
    windowTextMissing: (cause: string) =>
      WINDOW_TEXT_MISSING[cause] ??
      `No se pudo leer su texto (causa ${cause}); va solo la captura.`,
    windowTextRefused: (reason: string) => `Su texto no se adjuntó: ${reason}`,
    pages: (pages: number) => (pages === 1 ? "1 página" : `${pages} páginas`),
    view: "Ver",
    viewImage: (name: string) => `Ver ${name}`,
    open: "Abrir",
    reveal: "Mostrar en Finder",
    loading: "Cargando…",
    previous: "Anterior",
    next: "Siguiente",
    copy: "Copiar",
    copied: "✓ Copiado",
    close: "Cerrar",
  },
} as const;
