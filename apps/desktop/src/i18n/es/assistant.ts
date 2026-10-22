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
    inputLabel: "Mensaje",
    send: "Enviar",
    interrupt: "Interrumpir",
    clarifyLabel: "Respuesta",
    clarify: "Responder",
    // The hub's approval choices (hub-v1 «Choices and questions»).
    choices: {
      once: "Permitir una vez",
      session: "En esta conversación",
      always: "Siempre",
      deny: "No permitir",
    },
    questionOf: (position: number, total: number) => `Pregunta ${position} de ${total}`,
  },
  assistant: {
    ...esAssistant,
    transcribingVoice: "Transcribiendo tu voz…",

    transcribing: "Transcribiendo…",
    voiceFailure: voiceFailureText,
    audioUnavailable: "Audio no disponible",
    voiceProgress: "Progreso de la nota de voz",
    voiceNoteRecording: (elapsed: string) =>
      `Grabando nota de voz, ${elapsed}. Pulsa para enviarla`,
    sendVoiceNote: "Enviar nota de voz",
    voiceNoteTooShort: (durationMs: number, minimumMs: number) =>
      `La nota de voz duró ${seconds(durationMs)} s y el mínimo es ${seconds(minimumMs)} s: no se ha enviado.`,
    retryTranscription: "Reintentar transcripción",

    loadMoreSessions: "Cargar más conversaciones",
    loadingSessions: "Cargando conversaciones…",
    sessionsFailed: "No se pudieron cargar las conversaciones.",
    retrySessions: "Reintentar",
    // What the agent did in a turn (hub-v1 chat.item).
    work: {
      label: "Lo que hizo",
      working: (duration: string) => `Trabajando · ${duration}`,
      worked: (duration: string) => `Trabajó ${duration}`,
      used: (subjects: string) => `Usó ${subjects}`,
      steps: (count: number) => (count === 1 ? "1 paso" : `${count} pasos`),
      reasoning: "Razonamiento",
      stepStatus: {
        pending: "Pendiente",
        in_progress: "En curso",
        completed: "Hecho",
        cancelled: "Cancelado",
      },
    },
    loadOlder: "Cargar mensajes anteriores",
    loadingOlder: "Cargando mensajes anteriores…",
    historyFailed: "No se pudieron cargar los mensajes.",
  },
} as const;

/** «1 minuto», «1 min 30 s», «2 minutos»: the silence that hangs a call up. */
function silenceText(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1_000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes === 0) return `${rest} s`;
  if (rest !== 0) return `${minutes} min ${rest} s`;
  return minutes === 1 ? "1 minuto" : `${minutes} minutos`;
}

/** What the composer says while it cannot send: why, and that the text stays. */
function keptWhile(reason: string): string {
  return `${reason} Lo que escribas se queda aquí hasta que puedas enviarlo.`;
}

/** Why the composer cannot send now; what is written stays. */
function blockedCopy(assistantName: string) {
  return {
    permission: keptWhile(`${assistantName} espera tu permiso.`),
    question: keptWhile(`${assistantName} espera tu respuesta.`),
    answering: keptWhile(`${assistantName} está respondiendo.`),
    transcribing: keptWhile("Se está transcribiendo tu nota de voz."),
    uploading: "Se está subiendo un adjunto; podrás enviar en cuanto termine.",
    offline: keptWhile("Sin conexión con tu hub."),
    starting: keptWhile("Orbit aún está conectando."),
    sending: "Enviando tu mensaje…",
  };
}

/** What the call says, in the agent's name. */
function callCopy(assistantName: string) {
  return {
    title: `Llamada con ${assistantName}`,
    speaking: `${assistantName} está hablando`,
    listening: "Te escucha",
    connecting: `Llamando a ${assistantName}…`,
    you: (question: string) => `Tú: ${question}`,
    seesScreen: "Puede ver tu pantalla",
    interrupt: "Interrúmpele hablando cuando quieras",
    seeConversation: "Ver la conversación",
    minimised: "Llamada minimizada",
    limits: (minutesLeft: number, silenceMs: number) =>
      `${minutesLeft < 1 ? "Queda menos de 1 min" : `Quedan ${minutesLeft} min`} · se cuelga sola tras ${silenceText(silenceMs)} sin que hables · ⌃⌥D cuelga desde cualquier app`,
  } as const;
}

export function assistantCopy(assistantName: string) {
  return {
    ...base,
    chat: { ...base.chat, roles: { assistant: assistantName } },
    assistant: {
      ...base.assistant,
      thinking: `${assistantName} está pensando…`,
      responding: `${assistantName} está respondiendo…`,
      noHermesVoice: `${assistantName} contesta por texto`,
      // The list of conversations.
      conversations: "Conversaciones",
      newShort: "Nueva",
      now: "Ahora",
      messageCount: (count: number) =>
        count === 0 ? "Sin mensajes" : count === 1 ? "1 mensaje" : `${count} mensajes`,
      // The header of the open conversation.
      waitsPermission: "espera tu permiso",
      asksSomething: "te hace una pregunta",
      working: "trabajando…",
      newConversationState: "conversación nueva",
      // The thread.
      emptyTitle: `Pregúntale a ${assistantName}`,
      emptyText: "Escribe abajo o graba una nota de voz con el micrófono.",
      thread: "Conversación",
      loadingAssistant: "Leyendo tus conversaciones del hub…",
      answerLabel: `Respuesta de ${assistantName}`,
      workingLabel: `${assistantName} trabajando`,
      yourMessage: "Tu mensaje",
      turnFailed: `${assistantName} no pudo responder. Vuelve a preguntar.`,
      askAgain: "Volver a preguntar",
      // The composer.
      composerLabel: `Mensaje para ${assistantName}`,
      composerPlaceholder: `Escribe a ${assistantName}…`,
      blocked: blockedCopy(assistantName),
      recordingNote: "Grabando nota de voz",
      escCancels: "· esc cancela",
      cancel: "Cancelar",
      voiceNoteButton: "Grabar nota de voz",
      // What the agent waits for.
      approvalTitle: `${assistantName} necesita tu permiso`,
      clarifyTitle: `${assistantName} te pregunta`,
      decided: {
        once: "Lo permitiste una vez.",
        session: "Lo permitiste en esta conversación.",
        always: "Lo permitiste siempre.",
        deny: "No lo permitiste.",
      } as Record<string, string>,
      stepWaits: "espera tu permiso",
      stepDenied: "no lo permitiste",
      // A Mac without its hub.
      unpaired: `Para hablar con ${assistantName}, empareja este Mac con tu hub.`,
      pairMac: "Emparejar este Mac",
      // «Editar y copiar».
      editorSubtitle: `Cámbiala y cópiala, o mándala a ${assistantName} como un mensaje nuevo.`,
    },
    call: callCopy(assistantName),
  } as const;
}
