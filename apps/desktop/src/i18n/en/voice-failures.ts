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
  voice_transcription_joined_next_turn: "Included with what you said next.",
  voice_transcription_call_turn_replaced: "Replaced by what you said next.",
  voice_transcription_call_stopped_automatically:
    "The call stopped automatically before transcription.",
  voice_transcription_helper_cut: "The Mac stopped capturing before transcription.",
  voice_transcription_capture_cut: "Capture was cut off before transcription.",
  voice_transcription_privacy_stopped: "You stopped Orbit before transcription.",
  voice_transcription_orbit_closing: "Orbit closed before transcription.",
  voice_transcription_generation_changed: "The conversation changed before transcription.",
  voice_transcription_slot_unavailable: "Another note was being transcribed.",
};

/** The numbers a failed transcription carries, in the order the user reads them. */
function failureNumbers(failure: VoiceFailure): string[] {
  const numbers = [`code ${failure.code}`];
  if (failure.hub_status != null) numbers.push(`hub ${numberFormat().format(failure.hub_status)}`);
  if (failure.provider_status != null)
    numbers.push(`provider ${numberFormat().format(failure.provider_status)}`);
  if (failure.timeout_ms != null)
    numbers.push(`limit ${numberFormat().format(failure.timeout_ms / 1000)} s`);
  if (failure.bytes != null && failure.max_bytes != null)
    numbers.push(`${failure.bytes} of ${failure.max_bytes} bytes`);
  if (failure.attempts != null && failure.attempts > 1)
    numbers.push(`${numberFormat().format(failure.attempts)} attempts`);
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
  if (failure.code === "voice_transcription_empty") return "No speech was heard in the voice note.";
  if (failure.code === "voice_hub_unreachable")
    return "Saved on this Mac. Sends automatically when the hub reconnects.";
  if (failure.code === "voice_waiting_turn")
    return "Saved on this Mac. Sends automatically when the current response finishes.";
  if (failure.code === "voice_hub_pairing_required")
    return "Saved on this Mac. Pair Orbit in Settings and it will send automatically.";
  if (
    failure.code === "transcription_hub_not_configured" ||
    failure.code === "transcription_hub_invalid_url"
  )
    return "Couldn’t transcribe. Pair Orbit in Settings and choose “Retry transcription”.";
  // wire/stored name kept for compatibility with 0.9.0
  if (failure.code === "voice_transcription_interrupted_by_mars")
    return "You interrupted transcription.";
  const cause = transcriptionRefusals[failure.code];
  if (cause) return `${cause} The note is still saved on this Mac. Try again.`;
  const stopped = STOPPED_BEFORE_TRANSCRIPTION[failure.code];
  if (stopped) return stopped;
  if (providerRefusedAudio(failure))
    return "The transcription service rejected this audio. Listen to it, then record it again or discard it if needed.";
  return "Couldn’t transcribe. The note is still saved on this Mac. Try again or discard it.";
}

/** The codes and numbers behind a failed note, for whoever looks closer. */
export function voiceFailureDetail(failure: VoiceFailure): string {
  return failureNumbers(failure).join(", ");
}
