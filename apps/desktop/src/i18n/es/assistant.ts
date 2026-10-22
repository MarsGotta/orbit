import { esAssistant } from "../es-assistant";

interface VoiceFailure {
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
  if (failure.hub_status != null) numbers.push(`hub ${failure.hub_status}`);
  if (failure.provider_status != null) numbers.push(`proveedor ${failure.provider_status}`);
  if (failure.timeout_ms != null) numbers.push(`límite ${failure.timeout_ms / 1000} s`);
  if (failure.bytes != null && failure.max_bytes != null)
    numbers.push(`${failure.bytes} de ${failure.max_bytes} bytes`);
  if (failure.attempts != null && failure.attempts > 1)
    numbers.push(`${failure.attempts} intentos`);
  return numbers;
}

/**
 * Why a voice note was not transcribed, as the user can act on it: what
 * to do when they can do something; otherwise its cause and its numbers,
 * so the line names what happened.
 */
function voiceFailureText(failure: VoiceFailure): string {
  if (failure.code === "voice_transcription_empty") return "No se oyó nada en la nota de voz.";
  if (
    failure.code === "voice_hub_pairing_required" ||
    failure.code === "transcription_hub_not_configured" ||
    failure.code === "transcription_hub_invalid_url"
  )
    return "No se pudo transcribir: empareja Orbit en Ajustes y graba otra nota.";
  // wire/stored name kept for compatibility with 0.9.0
  if (failure.code === "voice_transcription_interrupted_by_mars")
    return "Interrumpiste la transcripción.";
  const stopped = STOPPED_BEFORE_TRANSCRIPTION[failure.code];
  if (stopped) return stopped;
  return `No se pudo transcribir la nota de voz (${failureNumbers(failure).join(", ")}). Vuelve a grabarla.`;
}

const secondsFormat = new Intl.NumberFormat("es-ES", { maximumFractionDigits: 2 });

/** Milliseconds as seconds, the Spanish way: 90 is «0,09». */
function seconds(ms: number): string {
  return secondsFormat.format(ms / 1_000);
}

const base = {
  chat: {
    empty: "Aún no hay mensajes. Escribe o dicta tu pregunta.",
    inputLabel: "Mensaje",
    send: "Enviar",
    interrupt: "Interrumpir",
    approve: "Aprobar",
    reject: "Rechazar",
    clarifyLabel: "Respuesta",
    clarify: "Responder",
  },
  assistant: {
    ...esAssistant,
    transcribingVoice: "Transcribiendo tu voz…",

    transcribing: "Transcribiendo…",
    voiceFailure: voiceFailureText,
    audioUnavailable: "Audio no disponible",
    voiceProgress: "Progreso de la nota de voz",
    recordingVoiceNote: (elapsed: string) => `Grabando nota de voz · ${elapsed} · Esc cancela`,
    voiceNoteRecording: (elapsed: string) =>
      `Grabando nota de voz, ${elapsed}. Pulsa para enviarla`,
    sendVoiceNote: "Enviar nota de voz",
    voiceNoteTooShort: (durationMs: number, minimumMs: number) =>
      `La nota de voz duró ${seconds(durationMs)} s y el mínimo es ${seconds(minimumMs)} s: no se ha enviado.`,
    retryTranscription: "Reintentar transcripción",
    // Said while macOS grants Screen Recording and privacy is armed.
    watchingScreen: "Puede ver tu pantalla",

    loadMoreSessions: "Cargar más conversaciones",
    loadingSessions: "Cargando conversaciones…",
    sessionsFailed: "No se pudieron cargar las conversaciones.",
    retrySessions: "Reintentar",
    loadOlder: "Cargar mensajes anteriores",
    loadingOlder: "Cargando mensajes anteriores…",
    historyFailed: "No se pudieron cargar los mensajes.",
  },
} as const;

export function assistantCopy(assistantName: string) {
  return {
    ...base,
    chat: { ...base.chat, roles: { assistant: assistantName } },
    assistant: {
      ...base.assistant,
      thinking: `${assistantName} está pensando…`,
      responding: `${assistantName} está respondiendo…`,
      noHermesVoice: `${assistantName} contesta por texto`,
    },
  } as const;
}
