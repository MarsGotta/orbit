import type { Entries, NoticeData } from "./types";

/**
 * Why a capture was cut, by the code the daemon gives it: a stop cause's
 * name or the helper's own code (`privacy_stop.rs`). One this version does not
 * know is still named, by its code.
 */
const cutCauses: Record<string, string> = {
  helper_lost: "el ayudante de audio de Orbit dejó de responder",
  helper_disconnected: "el ayudante de audio de Orbit dejó de responder",
  daemon_heartbeat_expired: "Orbit tardó demasiado en responder al ayudante de audio",
  helper_capture_stop: "el ayudante de audio paró la captura",
  helper_integrity_failure: "el audio llegó dañado del ayudante de audio",
  capture_event_failed: "se perdió la continuidad del audio",
  session_inactive: "el Mac se bloqueó o cambió de usuario",
  audio_device_changed: "cambió el dispositivo de audio",
  voice_processing_state_lost: "macOS dejó de procesar la voz del micrófono",
  system_audio_stream_stopped: "macOS paró el audio de la otra app",
};

function cause(data: NoticeData): string {
  const code = data.cause;
  if (typeof code !== "string" || code === "") return "falló la captura de audio";
  return cutCauses[code] ?? `falló la captura de audio (código ${code})`;
}

function fragments(count: unknown): string | null {
  if (typeof count !== "number" || count <= 0) return null;
  return count === 1 ? "1 fragmento" : `${count} fragmentos`;
}

/** What was kept of a recording, and what to do while the meeting goes on. */
function kept(count: unknown): string {
  const saved = fragments(count);
  const what = saved === null ? "" : ` (${saved})`;
  return `Lo grabado hasta entonces está guardado en este Mac${what}. Empieza otra grabación si la reunión sigue.`;
}

function cutText(data: NoticeData): string {
  const why = cause(data);
  if (data.recording === true)
    return `La grabación se cortó porque ${why}. ${kept(data.fragments)}`;
  if (data.call === true) return `La llamada se cortó porque ${why}.`;
  if (data.voice_note === true) return `La nota de voz se cortó porque ${why}.`;
  return `La captura de audio se cortó porque ${why}.`;
}

function count(value: unknown): number {
  return typeof value === "number" && value > 0 ? value : 0;
}

function lostFilesText(data: NoticeData): string {
  const damaged = count(data.invalid);
  const missing = count(data.missing);
  const unfinished = count(data.incomplete);
  const lost = damaged + missing + unfinished;
  const parts = [
    damaged > 0 ? `${damaged} ${damaged === 1 ? "dañado" : "dañados"}` : null,
    missing > 0 ? `${missing} que ${missing === 1 ? "faltaba" : "faltaban"}` : null,
    unfinished > 0 ? `${unfinished} a medio escribir` : null,
  ].filter((part) => part !== null);
  const files = lost === 1 ? "1 archivo de audio" : `${lost} archivos de audio`;
  const which = parts.length > 0 ? ` (${parts.join(", ")})` : "";
  return `Al abrirse, Orbit no pudo recuperar ${files} de este Mac${which}; recuperó ${count(data.recovered)}.`;
}

function closedText(data: NoticeData): string {
  if (data.audio_unsaved !== true)
    return "Orbit se cerró antes de terminar su trabajo pendiente; lo que ya estaba guardado en este Mac se retoma al abrirla.";
  const why = data.reserve_exhausted === true ? ", porque se agotó la reserva de memoria" : "";
  return `Orbit se cerró antes de escribir en este Mac parte del audio de la última grabación${why}. Esa parte se perdió.`;
}

/**
 * What could not be recorded or saved, in words of its own: what happened,
 * its cause when the notice carries one, and what to do. Never the generic
 * failure with only a code to share.
 */
export const captureFailed: Entries<
  | "capture_cut"
  | "recording_seal_failed"
  | "recording_interrupted_by_restart"
  | "store_recovery_lost_files"
  | "closed_with_pending_work"
> = {
  capture_cut: { class: "failed", text: cutText },
  recording_seal_failed: {
    class: "failed",
    text: () =>
      "Parte del audio de la grabación no se pudo escribir en este Mac. Si el disco está lleno, libera espacio: Orbit lo vuelve a intentar mientras siga abierta.",
  },
  recording_interrupted_by_restart: {
    class: "failed",
    text: (data) => `Orbit se cerró en mitad de una grabación. ${kept(data.fragments)}`,
  },
  store_recovery_lost_files: { class: "failed", text: lostFilesText },
  closed_with_pending_work: { class: "failed", text: closedText },
};
