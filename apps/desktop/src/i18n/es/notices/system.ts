import { fileSize } from "../attachments";
import { openSettings, rearm, worded, type Entries, type NoticeData } from "./types";

/** A folder chosen in the panel that Orbit did not keep, with its cause and numbers. */
function folderNotAdded(data: NoticeData): string {
  const { code, cause, count, max, bytes, max_bytes: maxBytes, status } = data;
  if (code === "too_many" && typeof max === "number") {
    const asked = typeof count === "number" ? ` (sería la ${count}.ª)` : "";
    return `No se añadió la carpeta: ya hay ${max} carpetas, el máximo${asked}. Quita una para añadir otra.`;
  }
  if (cause === "path_too_long" && typeof bytes === "number" && typeof maxBytes === "number")
    return `No se añadió la carpeta: su ruta ocupa ${bytes} bytes y Orbit admite ${maxBytes}.`;
  if (cause === "bookmark_too_large" && typeof bytes === "number" && typeof maxBytes === "number")
    return `No se añadió la carpeta: el marcador que macOS hizo de ella ocupa ${bytes} bytes y Orbit guarda hasta ${maxBytes}.`;
  if (code === "storage_failed")
    return "No se añadió la carpeta: Orbit no pudo guardarla en este Mac. Vuelve a añadirla.";
  if (code === "assistant_folder_panel_busy")
    return "El panel para elegir una carpeta ya está abierto.";
  const why = [code, cause, typeof status === "number" ? `error ${status}` : undefined]
    .filter((part) => part !== undefined)
    .join(", ");
  return `No se pudo añadir la carpeta: el panel de macOS no la entregó${why ? ` (${why})` : ""}. Vuelve a intentarlo.`;
}

/**
 * The hub's storage, nearly full or full: how much it uses of its limit,
 * what is left, what to do (free room, or raise the hub's quota when that
 * sets the limit) and what happens if nobody does. The hub deletes nothing.
 */
function hubStorage(data: NoticeData): string {
  const { bytes: used, max_bytes: max, remaining: left, cause } = data;
  const known = typeof used === "number" && typeof max === "number" && typeof left === "number";
  const state =
    known && left === 0
      ? `El hub está lleno: ocupa ${fileSize(used)} de ${fileSize(max)}.`
      : known
        ? `El hub está casi lleno: ocupa ${fileSize(used)} de ${fileSize(max)} y quedan ${fileSize(left)}.`
        : "El hub está casi lleno.";
  const fix =
    cause === "quota"
      ? "Libera espacio en el servidor o sube storage_quota_bytes en la configuración del hub."
      : "Libera espacio en el disco del servidor.";
  return `${state} ${fix} Si se llena, las reuniones nuevas se quedan en este Mac sin transcribir hasta que haya sitio. El hub no borra nada por su cuenta.`;
}

/** Privacy, the hub link and pairing, the local store, transcripts and archives. */
export const system = {
  engine_exited: {
    class: "fixable",
    text: (data) => {
      const process =
        data.cause === "proxy"
          ? "El proceso de conexión del motor de control"
          : "El motor de control";
      const status =
        typeof data.status !== "number"
          ? "sin comunicar el estado de salida"
          : data.code === "signal"
            ? `señal ${data.status}`
            : data.code === "exit"
              ? `código ${data.status}`
              : `estado de espera ${data.status}`;
      return `${process} terminó sin que Orbit lo pidiera (${status}). Orbit volverá a iniciarlo. No se repetirá ninguna acción de entrada.`;
    },
  },
  privacy_stopped: worded("done", "Todo está parado.", rearm),
  privacy_stop_kept: worded("fixable", "Todo sigue parado, como lo dejaste.", rearm),
  privacy_stop_active: worded("fixable", "Todo está parado. Reactiva Orbit para continuar.", rearm),
  privacy_rearmed: worded("done", "✓ Orbit vuelve a funcionar"),
  device_revoked: worded(
    "fixable",
    "El hub retiró este Mac. Vuelve a emparejarlo en Ajustes.",
    openSettings,
  ),
  device_pairing_mismatch: worded(
    "fixable",
    "El emparejamiento con el hub no coincide. Vuelve a emparejar en Ajustes.",
    openSettings,
  ),
  hub_storage_nearly_full: { class: "fixable", text: hubStorage },
  orbit_closing: worded("working", "Cerrando Orbit…"),
  pending_work_keep_open: worded("fixable", "Orbit sigue guardando. No lo cierres todavía."),
  pairing_failed: worded(
    "fixable",
    "No se pudo emparejar. Comprueba el código y la dirección del hub.",
  ),
  paired: worded("done", "Mac emparejado."),
  conversations_need_hub: worded(
    "fixable",
    "Las conversaciones están en el hub y no hay conexión con él. Comprueba la conexión.",
  ),
  transcript_retranscribe_unconfirmed: {
    class: "fixable",
    text: (data) => {
      let cause = "no se pudo conectar con el hub";
      if (data.cause === "hub_response_too_large")
        cause = "la respuesta del hub ocupa más de lo que Orbit lee";
      if (data.cause === "hub_response_cut") cause = "la respuesta del hub se interrumpió";
      if (data.cause === "hub_response_invalid")
        cause = "la respuesta del hub no tiene el formato esperado";
      if (data.cause === "hub_status") cause = "el hub no está disponible en este momento";
      return `El hub no ha confirmado la nueva transcripción: ${cause}. Puede seguir en curso.`;
    },
  },
  transcript_retranscribe_refused: {
    class: "fixable",
    text: (data) => {
      if (data.code === "recording_still_open")
        return "No se puede rehacer la transcripción: la grabación todavía no se ha cerrado.";
      if (data.code === "diarization_not_configured")
        return "No se puede rehacer la transcripción: el hub no tiene configurada la separación de voces.";
      return `El hub rechazó rehacer la transcripción.${data.code ? ` (${data.code})` : ""}`;
    },
  },
  transcript_retry_requested: worded("done", "Reintentando la transcripción."),
  note_change_unsent: worded("fixable", "No se guardó el cambio: no hay conexión con el hub."),
  turn_no_longer_exists: {
    class: "fixable",
    text: (data) => {
      const at =
        typeof data.at_ms === "number"
          ? ` (${Math.floor(data.at_ms / 60_000)}:${String(Math.floor(data.at_ms / 1_000) % 60).padStart(2, "0")})`
          : "";
      const turns = typeof data.turns === "number" ? ` y ahora tiene ${data.turns} turnos` : "";
      return `Ese turno${at} ya no está: la transcripción cambió desde que la abriste${turns}. Ábrela de nuevo.`;
    },
  },
  assistant_folder_not_added: { class: "fixable", text: folderNotAdded },
  meeting_audio_unreadable: {
    class: "fixable",
    text: (data) =>
      typeof data.count === "number" && typeof data.total === "number"
        ? `No se puede rehacer la transcripción: el audio de ${data.count} de ${data.total} grupos no se puede leer.`
        : "No se puede rehacer la transcripción: parte del audio no se puede leer.",
  },
} satisfies Partial<Entries>;
