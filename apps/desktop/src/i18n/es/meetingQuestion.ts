import { numberFormat } from "../locale";
// Deimos during the meeting — «Preguntar a Deimos», its answer in
// the floating panel, and «Oye Deimos». A question without answer
// says what happened, with the limits and recovery that apply.
const base = {
  meetingQuestion: {
    askAgain: "Otra pregunta",

    placeholder: "¿Qué dijo Lucía del presupuesto?",
    windowLabel: "Contexto de la reunión",
    window: (minutes: number) => `Últimos ${numberFormat().format(minutes)} min`,
    send: "Preguntar",
    cancel: "Cancelar",
    close: "Cerrar",

    asked: (question: string) => `Tu pregunta: ${question}`,

    linkWithHost: (label: string, host: string) => `${label} (${host})`,
    linkRefused: "No se pudo abrir el enlace.",
    causes: {
      privacyStopped: "Todo está parado. Reactiva Orbit para preguntar.",
      noMeeting: "No hay ninguna grabación en curso.",
      hubUnreachable: "Sin conexión con el hub. Pregunta de nuevo cuando vuelva.",
      tryAgain:
        "La respuesta no está disponible y Orbit no recibió la causa. Revisa la conversación antes de volver a preguntar.",
      questionEmpty: "La pregunta está vacía.",
      questionTooLong: (bytes: number, max: number) =>
        `La pregunta ocupa ${numberFormat().format(bytes)} bytes y el límite es ${numberFormat().format(max)} bytes. Acórtala.`,
      windowNotOffered: (minutes: number, offered: number[]) =>
        `La ventana de ${numberFormat().format(minutes)} min solicitada no está disponible. Elige entre los minutos ofrecidos: ${offered.map((n) => numberFormat().format(n)).join(", ") || "ninguno"}.`,
      turnFailed:
        "El turno del asistente falló. Revisa la conversación antes de volver a preguntar.",
      noAnswer:
        "El asistente terminó sin una respuesta. Revisa la conversación o reformula la pregunta.",
      unavailable: (code: string) =>
        `El asistente no está disponible (${code}). Pide a quien gestiona tu hub que lo revise.`,
      hubRefused: (code: string, message: string) =>
        `El hub rechazó la pregunta (${code})${message ? `: ${message}` : ". No indicó más detalles."}`,
      sendFailed: (message: string) =>
        `La pregunta no se pudo enviar. Comprueba la conexión con el hub antes de intentarlo de nuevo.${message ? ` Detalles: ${message}` : ""}`,
      refusalCodes: {
        upstream_unavailable:
          "El asistente está desconectado del hub. Pregunta de nuevo cuando vuelva.",
        upstream_rejected:
          "El asistente rechazó la solicitud. Pide a quien gestiona tu hub que revise su configuración.",
        upstream_protocol:
          "El asistente envió una respuesta incompatible. Pide a quien gestiona tu hub que revise las versiones del hub y del asistente.",
        turn_outcome_unknown:
          "El hub no puede confirmar qué pasó con la pregunta anterior. Revisa la conversación antes de volver a preguntar.",
      },
      interrupted: "Interrumpiste la respuesta.",
      revoked: "El hub retiró este Mac. Vuelve a emparejarlo en Ajustes.",
    },
    busy: {
      voice_note: "Estás grabando una nota de voz.",
      voice_transcription: "Aún se transcribe tu nota de voz.",
    },
    voice: {
      why: {
        privacyStopped: "todo está parado",
        paused: "la grabación está en pausa",
        macosVersion: "necesita macOS 26; usa el botón",
        localeUnsupported: "macOS no ofrece reconocimiento de voz en español; usa el botón",
        unavailable: "este Mac no lo admite; usa el botón",
        failed: "el reconocimiento de voz se detuvo; usa el botón",
        helperDisconnected: "el reconocimiento de voz no está disponible; usa el botón",
        helperWithoutPhrase: "actualiza Orbit",
        noMicrophone: "la grabación aún no tiene micrófono",
      },
    },
  },
} as const;

export function meetingQuestionCopy(assistantName: string) {
  return {
    ...base,
    meetingQuestion: {
      ...base.meetingQuestion,
      ask: `Preguntar a ${assistantName}`,
      fieldLabel: `Pregunta para ${assistantName}`,
      panelLabel: `${assistantName} en la reunión`,
      thinking: `${assistantName} está pensando…`,
      responding: `${assistantName} está respondiendo…`,
      alsoInConversation: `Queda también en tu conversación con ${assistantName}`,
      failed: `${assistantName} no respondió`,
      notSent: `La pregunta no llegó a ${assistantName}`,
      busy: {
        ...base.meetingQuestion.busy,
        assistant_turn: `${assistantName} aún responde a lo anterior. Espera o interrúmpelo.`,
        pending_approval: `${assistantName} espera tu respuesta en la conversación.`,
        call: `Hay una llamada con ${assistantName} abierta.`,
      },
      voice: {
        ...base.meetingQuestion.voice,
        listening: `«Oye ${assistantName}» te escucha`,
        downloading: (percent: number) =>
          `«Oye ${assistantName}» aún no escucha: descargando el reconocimiento de voz (${numberFormat().format(percent)} %)`,
        off: (why: string) => `«Oye ${assistantName}» no escucha: ${why}`,
        // «Oye …» was heard and no question followed: why, and what to do.
        abandoned: {
          phrase_revised: `No entendí bien «Oye ${assistantName}». Repítelo con la pregunta seguida.`,
          question_wait_expired: `Oí «Oye ${assistantName}», pero no llegó ninguna pregunta. Dila justo después del nombre.`,
          question_too_long: `Oí «Oye ${assistantName}», pero no distinguí ninguna pregunta. Prueba con «Preguntar a ${assistantName}».`,
        },
        short: {
          listening: `«Oye ${assistantName}» escucha`,
          downloading: (percent: number) =>
            `«Oye ${assistantName}» ${numberFormat().format(percent)} %`,
          off: `«Oye ${assistantName}» no escucha`,
        },
      },
    },
  } as const;
}
