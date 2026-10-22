import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { message } from "./chat";
import { NOW } from "./time";

/** The steps before the approval: two done, the third waits for it. */
export const approvalWork: AgentWork = {
  turn_id: "m7",
  started_at_ms: NOW - 42_000,
  tools: 2,
  subjects: ["tus notas"],
  items: [
    {
      id: "a1",
      kind: "tool",
      phase: "completed",
      label: "Leer la nota «Hoteles Ribera · arranque del piloto»",
      steps: [],
    },
    {
      id: "a2",
      kind: "tool",
      phase: "completed",
      label: "Escribir el borrador para Pablo Medina",
      steps: [],
    },
    { id: "a3", kind: "tool", phase: "started", label: "Enviar el correo desde Mail", steps: [] },
  ],
};

/** The turn once the approval is answered: sent, or not. */
export function approvalAnswered(allowed: boolean): {
  work: AgentWork;
  answer: ChatMessage;
} {
  return {
    work: {
      ...approvalWork,
      ended_at_ms: NOW - 42_000 + 46_000,
      tools: allowed ? 3 : 2,
      subjects: allowed ? ["Mail"] : ["tus notas"],
      items: approvalWork.items.map((item) =>
        item.id === "a3"
          ? { ...item, phase: allowed ? ("completed" as const) : ("failed" as const) }
          : item,
      ),
    },
    answer: message({
      id: "m8",
      role: "assistant",
      text: allowed
        ? "Listo: el correo «Arranque del piloto el lunes 26» ya está enviado a Pablo."
        : "De acuerdo, no lo envío. El borrador queda en Borradores de Mail.",
      at_ms: NOW,
    }),
  };
}
