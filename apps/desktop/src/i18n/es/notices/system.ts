import { openSettings, rearm, worded, type Entries } from "./types";

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
  privacy_stopped: worded("done", "Privacidad detenida.", rearm),
  privacy_stop_kept: worded("fixable", "Orbit sigue detenido por tu parada de privacidad.", rearm),
  privacy_stop_active: worded(
    "fixable",
    "La privacidad está detenida. Rearma Orbit para continuar.",
    rearm,
  ),
  privacy_rearmed: worded("done", "Orbit rearmado."),
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
        return "No se puede volver a transcribir: la grabación todavía no se ha cerrado.";
      if (data.code === "diarization_not_configured")
        return "No se puede volver a transcribir: el hub no tiene configurada la separación de voces.";
      return `El hub rechazó la petición de volver a transcribir.${data.code ? ` (${data.code})` : ""}`;
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
  meeting_audio_unreadable: {
    class: "fixable",
    text: (data) =>
      typeof data.count === "number" && typeof data.total === "number"
        ? `No se puede transcribir de nuevo: el audio de ${data.count} de ${data.total} grupos no se puede leer.`
        : "No se puede transcribir de nuevo: parte del audio no se puede leer.",
  },
} satisfies Partial<Entries>;
