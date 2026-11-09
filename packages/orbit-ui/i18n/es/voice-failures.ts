import { numberFormat } from "../locale";
import { transcriptionRefusals } from "./transcription";

/** Why a voice note was not transcribed, in the words and weight the user reads. */

export interface VoiceFailure {
  code: string;
  provider_status?: number | null;
  hub_status?: number | null;
  timeout_ms?: number | null;
  bytes?: string | null;
  max_bytes?: string | null;
  attempts?: number;
}

/**
 * Why a note stopped before its transcription, when Orbit itself stopped it:
 * each cause in its own words, never «Vuelve a grabarla» for
 * something the user did not do wrong.
 */
const STOPPED_BEFORE_TRANSCRIPTION: Record<string, string> = {
  voice_transcription_joined_next_turn: "Va junto con lo que dijiste a continuación.",
  voice_transcription_call_turn_replaced: "Lo que dijiste a continuación la sustituyó.",
  voice_transcription_call_stopped_automatically: "La llamada se paró sola antes de transcribirla.",
  voice_transcription_helper_cut: "El Mac dejó de capturar antes de transcribirla.",
  voice_transcription_capture_cut: "La captura se cortó antes de transcribirla.",
  voice_transcription_privacy_stopped: "Paraste Orbit antes de transcribirla.",
  voice_transcription_orbit_closing: "Orbit se cerró antes de transcribirla.",
  voice_transcription_generation_changed: "La conversación cambió antes de transcribirla.",
  voice_transcription_slot_unavailable: "Otra nota se estaba transcribiendo.",
};

/** The numbers a failed transcription carries, in the order the user reads them. */
function failureNumbers(failure: VoiceFailure): string[] {
  const numbers = [`código ${failure.code}`];
  if (failure.hub_status != null) numbers.push(`hub ${numberFormat().format(failure.hub_status)}`);
  if (failure.provider_status != null)
    numbers.push(`proveedor ${numberFormat().format(failure.provider_status)}`);
  if (failure.timeout_ms != null)
    numbers.push(`límite ${numberFormat().format(failure.timeout_ms / 1000)} s`);
  if (failure.bytes != null && failure.max_bytes != null)
    numbers.push(`${failure.bytes} de ${failure.max_bytes} bytes`);
  if (failure.attempts != null && failure.attempts > 1)
    numbers.push(`${numberFormat().format(failure.attempts)} intentos`);
  return numbers;
}

/**
 * Whether the provider behind the hub refused the note's audio: sending the
 * same audio again would be refused again. A 0.11.0 hub said it as an HTTP
 * error whose provider answered 400.
 */
export function providerRefusedAudio(failure: VoiceFailure): boolean {
  return (
    failure.code === "transcription_provider_refused" ||
    (failure.code === "transcription_http_error" && failure.provider_status === 400)
  );
}

/**
 * Why a voice note was not transcribed, in a calm line the user can act on.
 * Nothing is lost while the note is there (its audio stays on this Mac), so
 * the line says what to do and leaves the codes to the detail
 * (`voiceFailureDetail`).
 */
export function voiceFailureText(failure: VoiceFailure): string {
  if (failure.code === "voice_transcription_empty") return "No se oyó nada en la nota de voz.";
  if (failure.code === "voice_hub_unreachable")
    return "Guardada en el Mac: se enviará sola cuando vuelva la conexión con el hub.";
  if (failure.code === "voice_waiting_turn")
    return "Guardada en el Mac: se enviará sola cuando termine la respuesta en curso.";
  if (failure.code === "voice_hub_pairing_required")
    return "Guardada en el Mac: empareja Orbit en Ajustes y se enviará sola.";
  if (
    failure.code === "transcription_hub_not_configured" ||
    failure.code === "transcription_hub_invalid_url"
  )
    return "No se pudo transcribir: empareja Orbit en Ajustes y pulsa «Reintentar transcripción».";
  // wire/stored name kept for compatibility with 0.9.0
  if (failure.code === "voice_transcription_interrupted_by_mars")
    return "Interrumpiste la transcripción.";
  const cause = Object.hasOwn(transcriptionRefusals, failure.code)
    ? transcriptionRefusals[failure.code]
    : undefined;
  if (cause) return `${cause} La nota sigue guardada en el Mac: reinténtalo.`;
  const stopped = Object.hasOwn(STOPPED_BEFORE_TRANSCRIPTION, failure.code)
    ? STOPPED_BEFORE_TRANSCRIPTION[failure.code]
    : undefined;
  if (stopped) return stopped;
  if (providerRefusedAudio(failure))
    return "El servicio de transcripción no aceptó este audio. Escúchala y, si hace falta, dilo otra vez o descártala.";
  return `No se pudo transcribir (${failure.code}). La nota sigue guardada en el Mac: reinténtalo o descártala.`;
}

/** The codes and numbers behind a failed note, for whoever looks closer. */
export function voiceFailureDetail(failure: VoiceFailure): string {
  return failureNumbers(failure).join(", ");
}
