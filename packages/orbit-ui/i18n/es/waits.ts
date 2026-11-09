import { numberFormat } from "../locale";
import type { FeatureId } from "../../bridge/generated";

/** What a feature is called when its first state does not arrive. */
const featureNames: Record<FeatureId, string> = {
  status: "el estado de Orbit",
  permissions: "los permisos de este Mac",
  agenda: "tu calendario",
  notes: "tus notas",
  note: "la nota",
  recording: "el estado de la grabación",
  assistant: "el asistente",
  control: "lo que el asistente hace en este Mac",
  briefing: "Hoy",
  local: "los ajustes de este Mac",
};

/** Whole seconds, as the screen says a duration («13 s»). */
function seconds(ms: number): number {
  return Math.max(1, Math.round(ms / 1_000));
}

/** A wait that went past its ceiling, said with its numbers; a first read on its way. */
export const waits = {
  readingConversation: "Leyendo la conversación…",
  exceeded: (ceilingMs: number, code: string) =>
    `No se pudo completar a tiempo: Orbit no contestó en ${numberFormat().format(seconds(ceilingMs))} s. Vuelve a intentarlo. Si se repite, comparte el código ${code} con quien gestiona tu hub.`,
  firstState: (feature: FeatureId, ceilingMs: number) =>
    `No se pudo leer ${featureNames[feature]} en ${numberFormat().format(seconds(ceilingMs))} s. Cierra Orbit y vuelve a abrirlo.`,
};
