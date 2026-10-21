import { macPermissions } from "../permissions";
import { systemPrivacy, worded, type Entries, type NoticeData } from "./types";

function minutes(data: NoticeData): string {
  return typeof data.minutes === "number" ? `${data.minutes} min` : "su máximo";
}

/** Recording, the Mac's audio and the helper. */
export const capture = {
  audio_route_recovered: {
    class: "done",
    text: (data) =>
      typeof data.elapsed_ms === "number"
        ? `Se perdió el audio ${data.elapsed_ms < 1_000 ? "menos de 1 s" : `${Math.round(data.elapsed_ms / 1_000)} s`} al cambiar la ruta de audio; la grabación sigue.`
        : "Se perdió el audio al cambiar la ruta de audio; no se pudo medir la duración del hueco. La grabación sigue.",
  },
  recording_finished: worded("done", "Grabación guardada."),
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
    "Solo se graba tu micrófono: hay varias apps de videollamada. Elige una en «Fuente de audio».",
  ),
  helper_not_running: worded("fixable", "Orbit.app no está en marcha. Ábrela para continuar."),
  audio_reserve_exhausted: worded(
    "fixable",
    "Se perdió parte del audio por falta de memoria. No cierres Orbit hasta que termine de guardar.",
  ),
} satisfies Partial<Entries>;
