import { ARES, DEIMOS, ERIS, PHOBOS, waitingHub, withAgent } from "../data/assistants";
import { at } from "../data/time";
import type { Scenario } from "../scenario";
import { assistants, addEris, addSteps } from "./assistants-kit";

// Each assistant's settings behind its gear, and «Añadir un asistente».

export const assistantsGearScenarios: Scenario[] = [
  assistants({
    id: "asistentes-ajustes-de-deimos",
    state: "Ajustes de Deimos",
    description:
      "El engranaje de Deimos: lo da el hub, se ve en solo lectura y no se puede quitar.",
    route: `#/assistant/${DEIMOS.id}/settings`,
    size: { width: 1440, height: 1800 },
    ready: "text=funciona con Hermes",
  }),

  assistants({
    id: "asistentes-ajustes-de-phobos",
    state: "Ajustes de Phobos",
    description:
      "Conexión, cuenta, «Ver mi plan» y «Cerrar sesión»; personalidad, qué puede usar, habilidades y «Quitar».",
    route: `#/assistant/${PHOBOS.id}/settings`,
    size: { width: 1440, height: 1800 },
    ready: "role=region[name='Habilidades']",
  }),

  assistants({
    id: "asistentes-ajustes-de-ares",
    state: "Ajustes de Ares: sesión caducada",
    description: "La conexión de Ares dice que su sesión caducó, con «Iniciar sesión en Claude».",
    route: `#/assistant/${ARES.id}/settings`,
    size: { width: 1440, height: 1800 },
    ready: "role=region[name='Habilidades']",
  }),

  assistants({
    id: "asistentes-ajustes-uso-del-plan",
    state: "Ajustes de Ares: uso del plan",
    description:
      "Con su plan de Claude, que da el uso: la barra en ámbar desde el 80 % y cuándo se renueva.",
    route: `#/assistant/${ARES.id}/settings`,
    views: (base) => ({
      ...base,
      ...withAgent(base, ARES.id, {
        account: "signed_in",
        usage: { used_percent: 84, window: "seven_day", resets_at_ms: at(5, 9, 0) },
      }),
    }),
    ready: "role=progressbar[name='Uso de tu plan']",
  }),

  assistants({
    id: "asistentes-ajustes-quitar",
    state: "Quitar a Phobos",
    description: "«Quitar a Phobos de Orbit» pregunta en su fila: sus conversaciones se quedan.",
    route: `#/assistant/${PHOBOS.id}/settings`,
    ready: "role=button[name='Quitar a Phobos de Orbit']",
    steps: [
      { click: "role=button[name='Quitar a Phobos de Orbit']" },
      { scroll: "text=¿Quitar a Phobos?" },
    ],
  }),

  assistants({
    id: "asistentes-ajustes-sesion-cerrada",
    state: "Sesión cerrada",
    description:
      "Tras «Cerrar sesión»: sigue en «Tus asistentes» con sus conversaciones y pide iniciarla otra vez.",
    route: `#/assistant/${PHOBOS.id}/settings`,
    agent: (call, views) =>
      call.call === "sign_out"
        ? {
            outcome: { outcome: "done" },
            views: withAgent(views, PHOBOS.id, { account: "signed_out" }),
          }
        : null,
    ready: "role=button[name='Cerrar sesión']",
    steps: [{ click: "role=button[name='Cerrar sesión']" }, { wait: "text=Sin sesión" }],
  }),

  assistants({
    id: "asistentes-anadir",
    state: "Añadir: paso 1",
    description:
      "«¿Con qué va a funcionar?»: tu suscripción de Claude, la de ChatGPT o Hermes en tu hub.",
    route: "#/assistant/add",
    ready: "text=¿Con qué va a funcionar?",
  }),

  assistants({
    id: "asistentes-anadir-esperando",
    state: "Añadir: iniciando sesión",
    description:
      "Paso 2 mientras el navegador está abierto: el código que da la página y «Cancelar».",
    route: "#/assistant/add",
    agent: (call, views) =>
      call.call === "add"
        ? { outcome: { outcome: "added", agent_id: ERIS.id }, views: addEris(views, "signed_out") }
        : waitingHub(call),
    ready: "text=¿Con qué va a funcionar?",
    steps: [
      { click: "role=button[name='Continuar']" },
      { click: "role=button[name='Iniciar sesión en Claude']" },
      { wait: "role=textbox[name='Código de la página']" },
    ],
  }),

  ...addSteps(),

  assistants({
    id: "asistentes-anadir-hermes",
    state: "Añadir: Hermes",
    description:
      "Un asistente de Hermes no se crea desde Orbit: «Pídeselo a quien gestiona tu hub».",
    route: "#/assistant/add",
    ready: "text=¿Con qué va a funcionar?",
    steps: [
      { click: "role=radio[name=/Hermes en tu hub/]" },
      { click: "role=button[name='Continuar']" },
    ],
  }),
];
