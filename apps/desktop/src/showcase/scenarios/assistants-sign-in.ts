import { ARES, ERIS, PHOBOS, waitingAttempt, waitingHub } from "../data/assistants";
import type { Scenario } from "../scenario";
import { assistants, fresh, roster, signsIn, failsWith } from "./assistants-kit";

// Signing an assistant in, its failures, and the plan's limit.

export const assistantsSignInScenarios: Scenario[] = [
  assistants({
    id: "asistentes-ares-sesion-caducada",
    state: "Ares: sesión caducada",
    description: "El aviso con «Iniciar sesión en Claude»; se puede escribir pero no enviar.",
    route: `#/assistant/${ARES.id}`,
    views: fresh(ARES.id),
    ready: "text=Inicia sesión otra vez en Claude",
    steps: [
      {
        fill: "role=textbox[name='Mensaje para Ares']",
        text: "Revisa el contrato de Ribera Playa",
      },
    ],
  }),

  assistants({
    id: "asistentes-ares-esperando",
    state: "Ares: esperando al navegador",
    description:
      "«Esperando a que inicies sesión en el navegador…» y el código que da la página de Claude.",
    route: `#/assistant/${ARES.id}`,
    views: fresh(ARES.id),
    agent: waitingHub,
    ready: "role=button[name='Iniciar sesión en Claude']",
    steps: [
      { click: "role=button[name='Iniciar sesión en Claude']" },
      { wait: "role=textbox[name='Código de la página']" },
    ],
  }),

  assistants({
    id: "asistentes-ares-sesion-iniciada",
    state: "Ares: sesión iniciada",
    description: "Ya conectado: el aviso se va y queda como quién y con qué plan.",
    route: `#/assistant/${ARES.id}`,
    views: fresh(ARES.id),
    agent: signsIn(),
    ready: "role=button[name='Iniciar sesión en Claude']",
    steps: [
      { click: "role=button[name='Iniciar sesión en Claude']" },
      { wait: "text=✓ Conectado" },
    ],
  }),

  assistants({
    id: "asistentes-codex-codigo",
    state: "Phobos: el código de Codex",
    description: "Codex da un código de un solo uso que se escribe en su página.",
    route: `#/assistant/${PHOBOS.id}`,
    views: fresh(PHOBOS.id, roster(PHOBOS.id, { account: "signed_out" })),
    agent: waitingHub,
    ready: "role=button[name='Iniciar sesión en ChatGPT']",
    steps: [
      { click: "role=button[name='Iniciar sesión en ChatGPT']" },
      { wait: "text=/ABCD-1234/" },
    ],
  }),

  assistants({
    id: "asistentes-fallo-cancelado",
    state: "Iniciar sesión: cancelado",
    description: "Cancelado a mitad: el asistente queda como estaba.",
    route: `#/assistant/${ARES.id}`,
    views: fresh(ARES.id),
    agent: (call) =>
      call.call === "sign_in_cancel"
        ? {
            outcome: {
              outcome: "sign_in",
              attempt: { ...waitingAttempt(call.agent_id), state: "cancelled" },
            },
          }
        : waitingHub(call),
    ready: "role=button[name='Iniciar sesión en Claude']",
    steps: [
      { click: "role=button[name='Iniciar sesión en Claude']" },
      { click: "role=status >> role=button[name='Cancelar']" },
      { wait: "text=/Cancelaste el inicio de sesión/" },
    ],
  }),

  assistants({
    id: "asistentes-fallo-sin-codigo",
    state: "Iniciar sesión: la cuenta no lo permite",
    description:
      "La cuenta de ChatGPT no deja iniciar sesión con un código: lo dice con la causa del hub.",
    route: `#/assistant/${PHOBOS.id}`,
    views: fresh(PHOBOS.id, roster(PHOBOS.id, { account: "signed_out" })),
    agent: failsWith("device_code_disabled"),
    ready: "role=button[name='Iniciar sesión en ChatGPT']",
    steps: [
      { click: "role=button[name='Iniciar sesión en ChatGPT']" },
      { wait: "role=button[name='Volver a intentarlo']" },
    ],
  }),

  assistants({
    id: "asistentes-fallo-sin-conexion",
    state: "Iniciar sesión: sin conexión",
    description: "Sin conexión con el hub: no se puede iniciar sesión y se dice por qué.",
    route: `#/assistant/${ARES.id}`,
    views: fresh(ARES.id),
    agent: (call) =>
      call.call === "sign_in"
        ? { outcome: { outcome: "failed", refusal: { code: "hub_unreachable" } } }
        : waitingHub(call),
    ready: "role=button[name='Iniciar sesión en Claude']",
    steps: [
      { click: "role=button[name='Iniciar sesión en Claude']" },
      { wait: "text=/No hay conexión con el hub/" },
    ],
  }),

  assistants({
    id: "asistentes-cuenta-repetida",
    state: "Iniciar sesión: cuenta de otro asistente",
    description:
      "La cuenta ya la usa otro asistente: se puede seguir, y se avisa de que gastan del mismo plan.",
    route: `#/assistant/${ARES.id}`,
    views: fresh(ARES.id, (views) => ({
      status: {
        ...views.status,
        settings: {
          ...views.status.settings,
          agents: [...views.status.settings.agents, { ...ERIS, name: "Eris" }],
        },
      },
    })),
    agent: signsIn(),
    ready: "role=button[name='Iniciar sesión en Claude']",
    steps: [
      { click: "role=button[name='Iniciar sesión en Claude']" },
      { wait: "text=/ya la usa Eris/" },
    ],
  }),
];
