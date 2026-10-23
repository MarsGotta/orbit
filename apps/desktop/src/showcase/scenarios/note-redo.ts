import { wordsLibrary } from "../data/note-words";
import type { Scenario } from "../scenario";
import { answered, opened, route, withPass } from "./note-words";
import { reuniones, tab, transcriptSteps } from "./notes";

// The annex's change 7: «Rehacer transcripción» and «Rehacer resumen», each
// with its question, its progress, its end and its failure.

export const noteRedoScenarios: Scenario[] = [
  reuniones({
    id: "nota-retranscribir",
    state: "Transcripción · ¿Rehacer la transcripción?",
    description: "La pregunta anclada al botón, con lo que usa y lo que se mantiene.",
    route,
    views: (base) => opened(base),
    ready: tab("Transcripción"),
    steps: [...transcriptSteps, { click: "role=button[name='Rehacer transcripción']" }],
  }),
  reuniones({
    id: "nota-retranscribir-cola",
    state: "Transcripción · En cola",
    description: "Pedida y aún sin empezar: la anterior se ve atenuada.",
    route,
    views: (base) => opened(base, withPass({ state: "queued", error: null })),
    ready: tab("Transcripción"),
    steps: [...transcriptSteps, { wait: "text=En cola para rehacer la transcripción." }],
  }),
  reuniones({
    id: "nota-retranscribiendo",
    state: "Transcripción · Transcribiendo otra vez",
    description: "La tarjeta con lo que queda según el hub; la anterior, atenuada y sin tocar.",
    route,
    views: (base) =>
      opened(
        base,
        withPass({
          state: "running",
          error: null,
          progress: { done_ms: 1_071_600, total_ms: 2_820_000, remaining_ms: 170_000 },
        }),
      ),
    ready: tab("Transcripción"),
    steps: [...transcriptSteps, { wait: "text=unos 3 min" }],
  }),
  reuniones({
    id: "nota-retranscrita",
    state: "Transcripción · Rehecha",
    description: "Al terminar: «Rehaz el resumen para que la nota la use», con «Rehacer resumen».",
    route,
    views: (base) => opened(base, withPass({ state: "completed", error: null })),
    react: (views, command) =>
      command.command === "transcript_retranscribe"
        ? [
            answered(views, wordsLibrary, withPass({ state: "running", error: null })),
            answered(views, wordsLibrary, withPass({ state: "completed", error: null })),
          ]
        : null,
    ready: tab("Transcripción"),
    steps: [
      ...transcriptSteps,
      { click: "role=button[name='Rehacer transcripción']" },
      { click: "role=dialog >> role=button[name='Rehacer transcripción']" },
      { wait: "text=Transcripción rehecha." },
    ],
  }),
  reuniones({
    id: "nota-retranscribir-fallo",
    state: "Transcripción · No se pudo rehacer",
    description: "Sin diseño propio: el motivo del hub, en ámbar, y la transcripción anterior.",
    route,
    views: (base) =>
      opened(base, {
        ...withPass({ state: "failed", error: "final_pass_timeout", timeout_ms: 600_000 }),
        transcript: {
          ...withPass({ state: "failed", error: "final_pass_timeout", timeout_ms: 600_000 })
            .transcript,
          unmatched_speaker_names: 2,
        },
      }),
    ready: tab("Transcripción"),
    steps: [...transcriptSteps, { wait: "text=se conserva la anterior" }],
  }),
  reuniones({
    id: "nota-rehacer-resumen",
    state: "Resumen · ¿Rehacer el resumen?",
    description: "«Rehacer resumen» en la barra del Resumen, con su pregunta.",
    route,
    views: (base) => opened(base),
    ready: "role=button[name='Rehacer resumen']",
    steps: [{ click: "role=button[name='Rehacer resumen']" }],
  }),
  reuniones({
    id: "nota-reescribiendo",
    state: "Resumen · Reescribiendo",
    description: "Después de la pregunta: se reescribe y la versión hecha a mano queda guardada.",
    route,
    views: (base) => opened(base),
    hold: (command) => command.command === "note_regenerate",
    ready: "role=button[name='Rehacer resumen']",
    steps: [
      { click: "role=button[name='Rehacer resumen']" },
      { click: "role=dialog >> role=button[name='Rehacer resumen']" },
      { wait: "text=Reescribiendo la nota" },
    ],
  }),
];
