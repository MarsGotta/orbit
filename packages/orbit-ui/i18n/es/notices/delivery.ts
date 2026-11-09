import { numberFormat } from "../../locale";
import { fileSize } from "../attachments";
import { openSettings, type Entries, NoticeData } from "./types";

/** Bytes as the hub's other storage notices say them: «10 GB». */
function size(bytes: unknown): string | undefined {
  return typeof bytes === "number" ? fileSize(bytes) : undefined;
}

/** «HTTP 409, recording_finalized»: the status and the hub's own code. */
function answer(data: NoticeData): string {
  const status =
    typeof data.status === "number" ? `HTTP ${numberFormat().format(data.status)}` : "sin estado";
  return typeof data.code === "string" ? `${status}, ${data.code}` : status;
}

function what(data: NoticeData): string {
  return data.step === "recording_finalize" ? "el cierre de una reunión" : "un trozo de audio";
}

/** Uploads to the hub: what it refused or could not store, and what to do. */
export const delivery = {
  upload_refused_retrying: {
    class: "fixable",
    text: (data) =>
      data.status === 404
        ? `El hub no encontró dónde recibir ${what(data)} (${answer(data)}). Se reintenta solo y el audio se queda en el Mac; si sigue, revisa la dirección del hub en Ajustes.`
        : `El hub no acepta este Mac (${answer(data)}). Vuelve a emparejarlo en Ajustes: el audio se queda en el Mac y se sube después.`,
    action: openSettings,
  },
  hub_storage_full: {
    class: "fixable",
    text: (data) => {
      const used = size(data.bytes);
      const max = size(data.max_bytes);
      const free = size(data.remaining);
      const numbers = [
        used && max ? `usa ${used} de ${max}` : used ? `usa ${used}` : undefined,
        free ? `quedan ${free}` : undefined,
      ]
        .filter((part) => part !== undefined)
        .join(", ");
      return `El hub no tiene espacio para más audio${numbers ? ` (${numbers})` : ""}. El audio se queda en el Mac y se sube cuando haya sitio: libera espacio en el hub o amplía su límite.`;
    },
  },
} satisfies Partial<Entries>;

/** A refusal the hub will repeat for the same bytes: said, never retried. */
export const uploadRefused: Entries<"upload_refused"> = {
  upload_refused: {
    class: "failed",
    text: (data) =>
      `El hub rechazó ${what(data)} (${answer(data)}). Se queda en el Mac y no se reintenta; la reunión se cierra sin él.`,
  },
};
