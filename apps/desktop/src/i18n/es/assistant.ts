import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { assistantBase } from "./assistantBase";
import { voiceFailureDetail, voiceFailureText } from "./voice-failures";

const secondsFormat = numberFormat({ maximumFractionDigits: 2 });

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
    questionOf: (position: number, total: number) =>
      `Pregunta ${numberFormat().format(position)} de ${numberFormat().format(total)}`,
  },
  assistant: {
    ...assistantBase,
    voiceNoteHelp:
      "Pulsa para grabar y otra vez para enviar; mantén pulsado para hablar y suelta para enviar.",
    transcribingVoice: "Transcribiendo tu voz…",

    transcribing: "Transcribiendo…",
    voiceFailure: voiceFailureText,
    voiceFailureDetail,
    audioUnavailable: "Audio no disponible",
    voiceProgress: "Progreso de la nota de voz",
    voiceNoteRecording: (elapsed: string) =>
      `Grabando nota de voz, ${elapsed}. Pulsa para enviarla`,
    sendVoiceNote: "Enviar nota de voz",
    voiceNoteTooShort: (durationMs: number, minimumMs: number) =>
      `La nota de voz duró ${seconds(durationMs)} s y el mínimo es ${seconds(minimumMs)} s: no se ha enviado.`,
    retryTranscription: "Reintentar transcripción",
    discardVoiceNote: "Descartar",
    discardVoiceNoteTitle: "¿Descartar esta nota de voz?",
    discardVoiceNoteText: "Sale de la conversación y su audio se borra de este Mac.",

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
      steps: (count: number) =>
        pluralRules().select(count) === "one" ? "1 paso" : `${numberFormat().format(count)} pasos`,
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
    limits:
      "Tras 30 s de silencio, te pregunta si seguir; sin respuesta, cuelga en 15 s · ⌃⌥D cuelga desde cualquier app",
  } as const;
}

export function assistantCopy(assistantName: string) {
  return {
    ...base,
    chat: { ...base.chat, roles: { assistant: assistantName } },
    assistant: {
      ...base.assistant,
      thinking: `${assistantName} está pensando…`,
      awaitingPermission: `${assistantName} espera tu permiso`,
      awaitingAnswer: `${assistantName} espera tu respuesta`,
      retryingOpenai: "OpenAI tarda en contestar; reintentando",
      retryingAnthropic: "Anthropic tarda en contestar; reintentando",
      responding: `${assistantName} está respondiendo…`,
      noHermesVoice: `${assistantName} contesta por texto`,
      // The list of conversations.
      conversations: "Conversaciones",
      newShort: "Nueva",
      now: "Ahora",
      messageCount: (count: number) =>
        count === 0
          ? "Sin mensajes"
          : pluralRules().select(count) === "one"
            ? "1 mensaje"
            : `${numberFormat().format(count)} mensajes`,
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
      // What was written before Orbit closed or fell, back in the box.
      draftRecovered: "Recuperado el borrador que no llegaste a enviar.",
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
      } as const,
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
