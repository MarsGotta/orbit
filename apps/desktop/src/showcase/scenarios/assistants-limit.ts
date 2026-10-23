import { DEIMOS, handedAnswer, RATES, ratesQuestion } from "../data/assistants";
import { message } from "../data/chat";
import type { Scenario } from "../scenario";
import { assistants, atLimit, limitNotice, sessionsWithRates } from "./assistants-kit";

// The plan's limit: «Seguir con Deimos», «Esperar a las 18:40» and the handover's line.

export const assistantsLimitScenarios: Scenario[] = [
  assistants({
    id: "asistentes-llega-al-limite",
    state: "Llega al límite",
    description:
      "La respuesta se para: «Seguir con Deimos», Ares pide iniciar sesión antes y «Esperar a las 18:40».",
    route: `#/assistant/c/${RATES}`,
    views: atLimit,
    hub: sessionsWithRates,
    later: limitNotice,
    ready: "text=/ha llegado al límite de tu plan de ChatGPT/",
  }),

  assistants({
    id: "asistentes-limite-sigue-con-deimos",
    state: "Sigue con Deimos",
    description:
      "Tras «Seguir con Deimos»: una línea marca el paso y la cabecera dice que sigue desde Phobos.",
    route: `#/assistant/c/${RATES}`,
    views: atLimit,
    hub: sessionsWithRates,
    later: limitNotice,
    react: (views, command) =>
      command.command === "handover"
        ? {
            assistant: {
              ...views.assistant,
              session_id: "s-sigue",
              agent_id: DEIMOS.id,
              turn: [message({ ...ratesQuestion, id: "h1" }), handedAnswer],
            },
          }
        : null,
    ready: "role=button[name='Seguir con Deimos']",
    steps: [
      { click: "role=button[name='Seguir con Deimos']" },
      { wait: "text=/le hemos pasado esta conversación/" },
    ],
  }),

  assistants({
    id: "asistentes-limite-esperar",
    state: "Esperar a las 18:40",
    description:
      "La pregunta espera al plan: se manda sola cuando Phobos vuelva a estar disponible.",
    route: `#/assistant/c/${RATES}`,
    views: atLimit,
    hub: sessionsWithRates,
    later: limitNotice,
    ready: "role=button[name='Esperar a las 18:40']",
    steps: [
      { click: "role=button[name='Esperar a las 18:40']" },
      { wait: "text=/Mandaremos tu pregunta entonces/" },
    ],
  }),
];
