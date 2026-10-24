import { duration } from "../duration";
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
  voice_transcription_failed: worded(
    "fixable",
    "No se pudo transcribir tu nota de voz. Sigue guardada: la tienes en la conversación.",
  ),
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
  // Ten minutes before a call's ceiling, as a recording's (`recording_ceiling_near`).
  call_ceiling_near: {
    class: "fixable",
    text: (data) => {
      const { remaining, minutes, max } = data;
      if (typeof remaining !== "number") {
        return "La llamada se colgará pronto: llega a su máximo. Si quieres seguir, empieza otra.";
      }
      const facts =
        typeof minutes === "number" && typeof max === "number"
          ? `: lleva ${duration(minutes)}, el máximo es ${duration(max)}`
          : "";
      return `La llamada se colgará en ${duration(remaining)}${facts}. Si quieres seguir, empieza otra.`;
    },
  },
  call_preference_unreadable: worded(
    "fixable",
    "No se pudo leer tu elección «Cuando empieces una videollamada». Elígela de nuevo en Ajustes.",
    openSettings,
  ),
  call_recording_started_automatically: worded("done", "Grabando la llamada, como elegiste."),
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
