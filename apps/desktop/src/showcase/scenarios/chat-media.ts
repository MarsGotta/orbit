import { emptyChat, SESSION } from "../data/chat";
import { attachmentsMessages, transcribingNote, voiceMessages } from "../data/chat-turns";
import type { HubData, Scenario } from "../scenario";
import { chat, withChat } from "./chat";

/** The open conversation holds `messages` and nothing else. */
const only =
  (messages: HubData["messages"][string]) =>
  (base: HubData): HubData => ({ ...base, messages: { [SESSION]: messages } });

const lastTurnDone = withChat({ work: undefined });

export const chatMediaScenarios: Scenario[] = [
  chat({
    id: "chat-adjuntos",
    state: "Mensaje con adjuntos",
    description: "Tu mensaje con sus imágenes en miniatura y el PDF encima, en su burbuja.",
    views: lastTurnDone,
    hub: only(attachmentsMessages),
    ready: "role=button[name='Ver Propuesta piloto Ribera.pdf']",
    steps: [{ wait: "img[alt='Pizarra reservas.png']" }],
  }),
  chat({
    id: "chat-visor-imagen",
    state: "Visor",
    description:
      "Una imagen a pantalla completa: nombre, tipo, tamaño, «1 de 3», acciones, flechas y puntos.",
    views: lastTurnDone,
    hub: only(attachmentsMessages),
    ready: "img[alt='Pizarra reservas.png']",
    steps: [
      { click: "role=button[name='Ver Pizarra reservas.png']" },
      { wait: "role=dialog >> img" },
    ],
  }),
  chat({
    id: "chat-visor-copiada",
    state: "Visor, imagen copiada",
    description:
      "«Copiar imagen» confirma «✓ Copiada»; «Abrir» dice «Abriendo…» mientras macOS lo abre.",
    views: lastTurnDone,
    hub: only(attachmentsMessages),
    ready: "img[alt='Pizarra reservas.png']",
    steps: [
      { click: "role=button[name='Ver ocupacion-semana-44.png']" },
      { wait: "role=dialog >> img" },
      { click: "role=dialog >> role=button[name='Copiar imagen']" },
      { wait: "role=dialog >> text=✓ Copiada" },
    ],
  }),
  chat({
    id: "chat-visor-pdf",
    state: "Visor, PDF",
    description: "El PDF adjunto en el visor, el tercero de la conversación.",
    views: lastTurnDone,
    hub: only(attachmentsMessages),
    ready: "role=button[name='Ver Propuesta piloto Ribera.pdf']",
    steps: [
      { click: "role=button[name='Ver Propuesta piloto Ribera.pdf']" },
      { wait: "role=dialog >> iframe" },
    ],
  }),
  chat({
    id: "chat-adjuntando",
    state: "Adjuntos en el mensaje que se escribe",
    description:
      "Dos archivos en fichas con su tamaño y «×», antes de enviarlos, con el texto escrito.",
    views: withChat(emptyChat),
    hub: (base) => ({ ...base, messages: {} }),
    ready: "role=button[name='Adjuntar']",
    steps: [
      { click: "role=button[name='Adjuntar']" },
      { wait: "role=group[name='Propuesta piloto Ribera.pdf']" },
      {
        fill: "role=textbox[name='Mensaje para Deimos']",
        text: "Revisa la propuesta y dime si la gráfica cuadra con el informe.",
      },
    ],
  }),
  chat({
    id: "chat-notas-de-voz",
    state: "Notas de voz",
    description:
      "Una nota transcrita, otra cuya transcripción falló con su causa, una que se transcribe y una demasiado corta.",
    views: (base) => ({
      ...lastTurnDone(base),
      assistant: {
        ...base.assistant,
        work: undefined,
        turn: [transcribingNote],
        activity: "transcribing",
        voice: { ...base.assistant.voice, too_short: { duration_ms: 420, minimum_ms: 800 } },
      },
    }),
    hub: only(voiceMessages),
    ready: "text=Reintentar transcripción",
  }),
  chat({
    id: "chat-grabando-nota",
    state: "Grabando una nota de voz",
    description:
      "La nota de voz que se graba: el tiempo, «esc cancela», «Cancelar» y «Enviar nota de voz».",
    views: (base) => ({
      ...lastTurnDone(base),
      assistant: {
        ...base.assistant,
        work: undefined,
        voice: { ...base.assistant.voice, state: "listening", level: 0.6 },
      },
    }),
    ready: "text=Grabando nota de voz",
    steps: [
      { wait: "text=Estado de los cinco hoteles" },
      { advance: 3_000 },
      { wait: "text=0:03" },
    ],
  }),
  chat({
    id: "chat-editor",
    state: "Editar y copiar",
    description: "Una respuesta en el editor: su texto a la izquierda y cómo se verá a la derecha.",
    views: lastTurnDone,
    ready: "text=Estado de los cinco hoteles",
    steps: [
      { click: "role=button[name='Editar y copiar'] >> nth=0" },
      { wait: "role=dialog[name='Editar la respuesta']" },
    ],
  }),
];
