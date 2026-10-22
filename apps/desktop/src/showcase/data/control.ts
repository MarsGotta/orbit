import type { Views } from "../scenario";
import { at } from "./time";

export const control: NonNullable<Views["control"]> = {
  availability: { state: "current" },
  operation_id: "",
  stage: "idle",
  detail: "",
  remaining_steps: 0,
  expires_in_ms: 0,
  engine: { state: "ready", tools: 14 },
  engine_tools: ["click", "type_text", "press_key", "scroll", "screenshot", "open_application"],
  history: [
    {
      operation_id: "op-31",
      requested: "Abrir el informe de ocupación",
      application: "Numbers",
      channel: "chat",
      at_ms: at(0, 9, 48),
      outcome: "performed",
    },
    {
      operation_id: "op-30",
      requested: "Copiar la tabla de tarifas",
      application: "Safari",
      channel: "chat",
      at_ms: at(-1, 18, 5),
      outcome: "performed",
    },
  ],
  folders: {
    state: "known",
    max: 8,
    folders: [
      { id: "f1", path: "/Users/lucia/Documents/Nubaria", access: { state: "accessible" } },
      { id: "f2", path: "/Users/lucia/Desktop/Piloto Ribera", access: { state: "accessible" } },
      {
        id: "f3",
        path: "/Volumes/Archivo/Contratos 2025",
        access: { state: "unavailable", cause: { kind: "access_denied", errno: 13 } },
      },
    ],
  },
};
