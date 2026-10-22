import { openSettings, worded, type Entries, type NoticeData } from "./types";

const asked: Record<string, string> = {
  screenshot_full: "una captura de la pantalla",
  screenshot_window: "una captura de una ventana",
  screen_sources: "la lista de ventanas",
  selected_text: "el texto que tienes seleccionado",
  control_tools: "las acciones que puede hacer en el Mac",
  perform_action: "hacer una acción en el Mac",
  calendar_read: "tu agenda",
};

function whatAssistantAsked(data: NoticeData, assistantName: string): string {
  const thing = typeof data.code === "string" ? asked[data.code] : undefined;
  return thing
    ? `${assistantName} ha pedido ${thing}.`
    : `${assistantName} ha pedido algo de este Mac.`;
}

/** Voice notes, the assistant, calls with Deimos and what Deimos asks of the Mac. */
const base = {
  voice_note_empty: worded("fixable", "No se grabó audio. Comprueba el micrófono."),
  voice_transcribing: worded("working", "Transcribiendo tu voz…"),
  voice_not_recognized: worded("fixable", "No se oyó nada. Vuelve a intentarlo."),
  assistant_session_not_found: worded(
    "fixable",
    "Esa conversación no está en este Mac. Escribe en una nueva.",
  ),
  assistant_turn_lost_by_restart: worded(
    "fixable",
    "Tu última pregunta quedó sin respuesta por un reinicio. Vuelve a preguntar.",
  ),

  assistant_interrupt_not_delivered: worded(
    "fixable",
    "La interrupción no llegó. Vuelve a interrumpir.",
  ),

  assistant_state_unknown: worded(
    "fixable",
    "El hub es más nuevo que esta versión de Orbit. Actualiza Orbit.",
    { label: "Buscar actualización", command: { command: "update_check" } },
  ),

  call_expired: worded("fixable", "La llamada llegó a su límite. Empieza otra si quieres seguir."),
  call_preference_unreadable: worded(
    "fixable",
    "No se pudo leer tu elección «Cuando empieces una videollamada». Elígela de nuevo en Ajustes.",
    openSettings,
  ),
  call_recording_started_automatically: worded("done", "Grabando la llamada, como elegiste."),
  call_recording_finished: worded("done", "Llamada terminada: grabación guardada."),
} satisfies Partial<Entries>;

export function conversationCopy(assistantName: string) {
  return {
    ...base,
    assistant_turn_lost_by_disconnect: worded(
      "fixable",
      `Se perdió la conexión mientras ${assistantName} respondía. Vuelve a preguntar.`,
    ),
    assistant_outcome_unknown: worded(
      "fixable",
      `No se sabe si ${assistantName} llegó a responder. Vuelve a preguntar si hace falta.`,
    ),
    assistant_interrupt_refused: worded(
      "fixable",
      `No se pudo interrumpir a ${assistantName}. Vuelve a intentarlo.`,
    ),
    approval_expired: worded(
      "fixable",
      `La petición de ${assistantName} ya había caducado y no se aplicó. Pídele que la repita.`,
    ),
    meeting_question_still_answering: worded(
      "fixable",
      `${assistantName} aún responde a tu pregunta anterior. Vuelve a preguntar cuando acabe.`,
    ),
    bridge_serving: {
      class: "done",
      text: (data: NoticeData) => whatAssistantAsked(data, assistantName),
    },
  } as const;
}
