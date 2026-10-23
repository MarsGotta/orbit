// Deimos during the meeting — «Preguntar a Deimos», its answer in
// the floating panel, and «Oye Deimos». A question without answer
// says what to do; its codes and sizes stay in the log.
const base = {
  meetingQuestion: {
    askAgain: "Otra pregunta",

    placeholder: "¿Qué dijo Lucía del presupuesto?",
    windowLabel: "Contexto de la reunión",
    window: (minutes: number) => `Últimos ${minutes} min`,
    send: "Preguntar",
    cancel: "Cancelar",
    close: "Cerrar",

    asked: (question: string) => `Tu pregunta: ${question}`,

    tooLong: "La pregunta es demasiado larga: acórtala.",

    linkWithHost: (label: string, host: string) => `${label} (${host})`,
    linkRefused: "No se pudo abrir el enlace.",
    causes: {
      privacyStopped: "Todo está parado. Reactiva Orbit para preguntar.",
      noMeeting: "No hay ninguna grabación en curso.",
      hubUnreachable: "Sin conexión con el hub. Pregunta de nuevo cuando vuelva.",
      tryAgain: "Vuelve a preguntar en un momento.",
      questionEmpty: "La pregunta está vacía.",
      questionTooLong: "La pregunta es demasiado larga: acórtala.",
      windowNotOffered: "Elige otra ventana de tiempo.",
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
        helperDisconnected: "Orbit.app no está en marcha",
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
          `«Oye ${assistantName}» aún no escucha: descargando el reconocimiento de voz (${percent} %)`,
        off: (why: string) => `«Oye ${assistantName}» no escucha: ${why}`,
        // «Oye …» was heard and no question followed: why, and what to do.
        abandoned: {
          phrase_revised: `No entendí bien «Oye ${assistantName}». Repítelo con la pregunta seguida.`,
          question_wait_expired: `Oí «Oye ${assistantName}», pero no llegó ninguna pregunta. Dila justo después del nombre.`,
          question_too_long: `Oí «Oye ${assistantName}», pero no distinguí ninguna pregunta. Prueba con «Preguntar a ${assistantName}».`,
        },
        short: {
          listening: `«Oye ${assistantName}» escucha`,
          downloading: (percent: number) => `«Oye ${assistantName}» ${percent} %`,
          off: `«Oye ${assistantName}» no escucha`,
        },
      },
    },
  } as const;
}
