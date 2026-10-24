import type { Scenario } from "../scenario";
import { at } from "../data/time";
import { notices, privacyStopped } from "./edit";

export const shellScenarios: Scenario[] = [
  {
    id: "privacidad-detenida",
    view: "Privacidad",
    state: "Todo parado",
    description:
      "Tras «Parar todo»: el aviso de la barra ofrece «Reactivar» y la agenda dice por qué no se lee.",
    route: "#/calendar",
    views: privacyStopped,
    ready: "role=status >> text=Todo está parado",
  },
  {
    id: "aviso-actualizacion",
    view: "Avisos",
    state: "Actualización disponible",
    description: "El aviso de una versión nueva, con su botón para instalarla.",
    route: "#/briefing",
    views: (base) => ({
      ...base,
      local: {
        ...base.local,
        update: { ...base.local.update, state: "available", version: "0.11.0" },
      },
    }),
    ready: "text=Hay una versión nueva de Orbit (0.11.0)",
  },
  {
    id: "avisos",
    view: "Avisos",
    state: "Varios a la vez",
    description:
      "Los avisos apilados: uno hecho, uno que se puede arreglar y un fallo con su código.",
    route: "#/briefing",
    later: (views) =>
      notices(
        views,
        { code: "recording_discarded", data: { recording: "rec-daily-21", at_ms: at(0, 9, 14) } },
        { code: "meeting_audio_microphone_only" },
        { code: "contained_failure", data: { step: "upload_jobs" } },
      ),
    ready: "text=Reunión descartada",
  },
];
