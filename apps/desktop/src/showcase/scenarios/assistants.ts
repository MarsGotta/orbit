import { DEIMOS, PHOBOS, withAgent } from "../data/assistants";
import type { Scenario } from "../scenario";
import { assistants, fresh, PENCIL } from "./assistants-kit";

// «Tus asistentes», «¿Con quién quieres hablar?», each one's page and the pencil.

export const assistantsScenarios: Scenario[] = [
  assistants({
    id: "asistentes-conversacion-nueva",
    state: "¿Con quién quieres hablar?",
    description:
      "«+ Nueva»: una tarjeta por asistente con su servicio, para qué sirve y su estado; «Hablar con» sobre el cuadro.",
    route: "#/assistant/new",
    views: fresh(DEIMOS.id),
    ready: "text=¿Con quién quieres hablar?",
  }),

  assistants({
    id: "asistentes-deimos",
    state: "Deimos, el principal",
    description:
      "Su conversación antes del primer mensaje: «Para empezar» y «Seguir donde lo dejaste».",
    route: `#/assistant/${DEIMOS.id}`,
    views: fresh(DEIMOS.id),
    ready: "text=Para empezar",
  }),

  assistants({
    id: "asistentes-phobos",
    state: "Phobos",
    description:
      "Pulsar Phobos en «Tus asistentes» abre su conversación; la cabecera, en la barra de arriba.",
    route: `#/assistant/${PHOBOS.id}`,
    views: fresh(PHOBOS.id),
    ready: "text=Para empezar",
  }),

  assistants({
    id: "asistentes-lapiz-editar",
    state: "Lápiz: editar",
    description: "El menú del lápiz: nombre, descripción con su contador, color y principal.",
    route: `#/assistant/${PHOBOS.id}`,
    views: fresh(PHOBOS.id),
    ready: PENCIL,
    steps: [{ click: PENCIL }, { wait: "role=dialog[name='Editar a Phobos']" }],
  }),

  assistants({
    id: "asistentes-lapiz-nombre-repetido",
    state: "Lápiz: nombre repetido",
    description:
      "Un nombre que ya tiene otro asistente, sin distinguir mayúsculas ni acentos: no se guarda.",
    route: `#/assistant/${PHOBOS.id}`,
    views: fresh(PHOBOS.id),
    ready: PENCIL,
    steps: [{ click: PENCIL }, { fill: "role=textbox[name='Nombre']", text: "Áres" }],
  }),

  assistants({
    id: "asistentes-lapiz-guardado",
    state: "Lápiz: guardado",
    description:
      "Guardado en el hub: la cabecera y «Tus asistentes» cambian y sale «✓ Guardado» 2,5 s.",
    route: `#/assistant/${PHOBOS.id}`,
    views: fresh(PHOBOS.id),
    agent: (call, views) =>
      call.call === "profile"
        ? {
            outcome: { outcome: "done" },
            views: withAgent(views, PHOBOS.id, { description: call.description }),
          }
        : null,
    ready: PENCIL,
    steps: [
      { click: PENCIL },
      { fill: "role=textbox[name='Descripción']", text: "Código, scripts y datos de reservas" },
      { click: "role=button[name='Guardar']" },
      { wait: "text=✓ Guardado" },
    ],
  }),
];
