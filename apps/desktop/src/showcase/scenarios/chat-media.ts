import { emptyChat, SESSION } from "../data/chat";
import { attachmentsMessages, voiceMessages } from "../data/chat-turns";
import { NOW } from "../data/time";
import type { HubData, Scenario, Views } from "../scenario";
import { chat } from "./chat";

/** The open conversation holds `messages` and nothing else. */
const only =
  (messages: HubData["messages"][string]) =>
  (base: HubData): HubData => ({ ...base, messages: { [SESSION]: messages } });

const lastTurnDone = (base: Views): Views => ({
  ...base,
  assistant: { ...base.assistant, work: undefined },
});

export const chatMediaScenarios: Scenario[] = [
  chat({
    id: "chat-adjuntos",
    state: "Mensaje con adjuntos",
    description:
      "Un mensaje con dos imágenes en miniatura y un PDF con sus páginas, tamaño y acciones.",
    views: lastTurnDone,
    hub: only(attachmentsMessages),
    ready: "role=group[name='Propuesta piloto Ribera.pdf']",
    steps: [{ wait: "img[alt='Pizarra reservas.png']" }],
  }),
  chat({
    id: "chat-visor-imagen",
    state: "Visor de imágenes",
    description: "Una imagen adjunta abierta en grande, con Copiar, Anterior y Siguiente.",
    views: lastTurnDone,
    hub: only(attachmentsMessages),
    ready: "img[alt='Pizarra reservas.png']",
    steps: [
      { click: "role=button[name='Ver Pizarra reservas.png']" },
      { wait: "role=dialog >> img" },
    ],
  }),
  chat({
    id: "chat-visor-pdf",
    state: "Visor de PDF",
    description: "Un PDF adjunto abierto en el visor de la app.",
    views: lastTurnDone,
    hub: only(attachmentsMessages),
    ready: "role=group[name='Propuesta piloto Ribera.pdf']",
    steps: [{ click: "role=button[name='Ver']" }, { wait: "role=dialog >> iframe" }],
  }),
  chat({
    id: "chat-adjuntando",
    state: "Adjuntos en el mensaje que se escribe",
    description:
      "Dos archivos adjuntados al mensaje en curso, antes de enviarlo, con el texto escrito.",
    views: (base) => ({ ...base, assistant: emptyChat }),
    hub: (base) => ({ ...base, messages: {} }),
    ready: "role=button[name='Adjuntar']",
    steps: [
      { click: "role=button[name='Adjuntar']" },
      { wait: "role=group[name='Propuesta piloto Ribera.pdf']" },
      {
        fill: "role=textbox[name='Mensaje']",
        text: "Revisa la propuesta y dime si la gráfica cuadra con el informe.",
      },
    ],
  }),
  chat({
    id: "chat-notas-de-voz",
    state: "Notas de voz",
    description:
      "Una nota de voz transcrita, otra cuya transcripción falló con su causa y una demasiado corta.",
    views: (base) => ({
      ...lastTurnDone(base),
      assistant: {
        ...base.assistant,
        work: undefined,
        voice: { ...base.assistant.voice, too_short: { duration_ms: 420, minimum_ms: 800 } },
      },
    }),
    hub: only(voiceMessages),
    ready: "text=Reintentar transcripción",
  }),
  chat({
    id: "chat-grabando-nota",
    state: "Grabando una nota de voz",
    description: "El micrófono graba una nota de voz: el tiempo que lleva y cómo cancelarla.",
    views: (base) => ({
      ...lastTurnDone(base),
      assistant: {
        ...base.assistant,
        work: undefined,
        voice: { ...base.assistant.voice, state: "listening", level: 0.6 },
      },
    }),
    ready: "text=/Grabando nota de voz/",
    steps: [
      { wait: "text=Estado de los cinco hoteles" },
      { advance: 7_000 },
      { wait: "text=/0:07/" },
      { scroll: "footer:has(textarea)", align: "end" },
    ],
  }),
  chat({
    id: "chat-editor",
    state: "Editor de una respuesta",
    description: "Una respuesta abierta en el editor para cambiarla y copiarla.",
    ready: "text=Estado de los cinco hoteles",
    steps: [
      { click: "role=button[name='Abrir en el editor']" },
      { wait: "role=dialog[name='Editar la respuesta']" },
    ],
  }),
  chat({
    id: "chat-llamada",
    state: "Llamada con Deimos",
    description:
      "Una llamada de voz en curso: la burbuja con lo último que se dijo y si ve la pantalla.",
    views: (base) => ({
      ...base,
      recording: {
        ...base.recording,
        call: {
          started_at_ms: NOW - 95_000,
          seeing_screen: true,
          last_line: "Ribera Aeropuerto va 16 puntos por debajo de septiembre.",
          speaking: true,
          expires_at_ms: NOW - 95_000 + 60 * 60_000,
          origin: "app",
          idle_hangup_ms: 60_000,
        },
      },
    }),
    ready: "text=/Ribera Aeropuerto va 16 puntos/",
  }),
];
