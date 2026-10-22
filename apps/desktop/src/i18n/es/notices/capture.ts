import { macPermissions } from "../permissions";
import { duration as span } from "../duration";
import { systemPrivacy, worded, type Entries, type NoticeData } from "./types";

function minutes(data: NoticeData): string {
  return typeof data.minutes === "number" ? span(data.minutes) : "su máximo";
}

function ceilingNear(data: NoticeData): string {
  const { remaining, minutes: recorded, max } = data;
  if (typeof remaining !== "number") {
    return "La grabación se parará pronto: llega a su máximo. Si la reunión sigue, termínala y empieza otra.";
  }
  const facts =
    typeof recorded === "number" && typeof max === "number"
      ? `: lleva ${span(recorded)}, el máximo es ${span(max)}`
      : "";
  return `La grabación se parará en ${span(remaining)}${facts}. Si la reunión sigue, termínala y empieza otra.`;
}

/** Why a cut happened, in the person's words; an unknown code is shown as it is. */
const CUT_CAUSES: Record<string, string> = {
  helper_lost: "se cerró el ayudante de audio de Orbit",
  daemon_heartbeat_expired: "el servicio de Orbit dejó de responder",
  capture_event_failed: "un fragmento de audio llegó mal",
  capture_maintenance_failed: "falló el guardado del audio",
  helper_integrity_failure: "el Mac no pudo garantizar el audio",
  audio_stop_unconfirmed: "el Mac no confirmó una parada del audio",
  capture_stop_undelivered: "una orden de parada no llegó al Mac",
  capture_finalize_timed_out: "el Mac no confirmó el final de la grabación",
  audio_device_changed_voice_note: "cambió el dispositivo de audio durante una nota de voz",
  voice_processing_state_lost: "el Mac perdió el procesado de voz",
  session_inactive: "el Mac se durmió o cambió de usuario",
  privacy_stop: "la parada de privacidad",
  restart: "Orbit se reinició",
  cause_not_saved: "una causa que no quedó guardada",
};

function cutCause(data: NoticeData): string {
  const code = typeof data.cause === "string" ? data.cause : undefined;
  if (code === undefined) return "una causa desconocida";
  return CUT_CAUSES[code] ?? code;
}

function missing(data: NoticeData): string {
  if (typeof data.elapsed_ms !== "number") return "";
  const seconds = Math.round(data.elapsed_ms / 1_000);
  return seconds < 1 ? "; falta menos de 1 s de audio" : `; faltan ${seconds} s de audio`;
}

/** Recording, the Mac's audio and the helper. */
export const capture = {
  recording_resumed_after_cut: {
    class: "done",
    text: (data) => `La grabación se cortó (${cutCause(data)}) y ha seguido sola${missing(data)}.`,
  },
  recording_closed_after_cut: {
    class: "fixable",
    text: (data) =>
      data.cause === "capture_finalize_timed_out"
        ? "La grabación terminó sin que el Mac confirmara su final: se ha cerrado con lo grabado y su nota se escribirá."
        : `La grabación se cortó (${cutCause(data)}) y no pudo seguir. Se ha cerrado con lo grabado y su nota se escribirá; si la reunión sigue, empieza otra grabación.`,
  },
  audio_route_recovered: {
    class: "done",
    text: (data) =>
      typeof data.elapsed_ms === "number"
        ? `Se perdió el audio ${data.elapsed_ms < 1_000 ? "menos de 1 s" : `${Math.round(data.elapsed_ms / 1_000)} s`} al cambiar la ruta de audio; la grabación sigue.`
        : "Se perdió el audio al cambiar la ruta de audio; no se pudo medir la duración del hueco. La grabación sigue.",
  },
  voice_note_cancelled_by_audio_route: worded(
    "fixable",
    "Cambió el dispositivo de audio mientras dictabas: la nota de voz se canceló y queda guardada sin enviar. La reunión sigue grabándose.",
  ),
  track_stopped: {
    class: "fixable",
    text: (data) =>
      data.origin === "mic_local"
        ? "Tu micrófono dejó de grabarse y no se pudo recuperar; el audio de la reunión sigue grabándose."
        : "El audio de la otra app dejó de grabarse y no se pudo recuperar; tu micrófono sigue grabando.",
  },
  recording_finished: worded("done", "Grabación guardada."),
  recording_ceiling_near: { class: "fixable", text: ceilingNear },
  recording_ceiling_reached: {
    class: "fixable",
    text: (data) =>
      `La grabación llegó a su máximo de ${minutes(data)}. Empieza otra si la reunión sigue.`,
  },
  microphone_permission_missing: worded(
    "fixable",
    "Falta el permiso de micrófono. Concédelo en Ajustes del Sistema.",
    systemPrivacy(macPermissions.microphone.pane),
  ),
  screen_permission_missing: worded(
    "fixable",
    "Falta el permiso de grabación de pantalla. Concédelo en Ajustes del Sistema.",
    systemPrivacy(macPermissions.screen_recording.pane),
  ),
  meeting_audio_microphone_only: worded(
    "fixable",
    "Solo se graba tu micrófono: no hay ninguna app de videollamada abierta.",
  ),
  meeting_audio_ambiguous: worded(
    "fixable",
    "Solo se graba tu micrófono: hay varias apps de videollamada. Elige una en «Qué audio grabar».",
  ),
  helper_not_running: worded("fixable", "Orbit.app no está en marcha. Ábrela para continuar."),
  audio_reserve_exhausted: worded(
    "fixable",
    "Se perdió parte del audio por falta de memoria. No cierres Orbit hasta que termine de guardar.",
  ),
} satisfies Partial<Entries>;
