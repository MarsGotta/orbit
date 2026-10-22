import type { Scenario } from "../scenario";

const section = (name: string) => `section[aria-label="${name}"]`;

const settings = (scenario: Omit<Scenario, "view" | "route" | "ready"> & { ready?: string }) => ({
  view: "Ajustes",
  route: "#/settings",
  ready: "role=heading[name='Ajustes']",
  ...scenario,
});

export const settingsScenarios: Scenario[] = [
  settings({
    id: "ajustes",
    state: "Arriba de la página",
    description:
      "El principio de Ajustes: el micrófono, qué hacer con las llamadas y lo que Deimos hizo en este Mac.",
  }),
  settings({
    id: "ajustes-agentes",
    state: "Agentes · uno de Hermes",
    description:
      "Deimos, un agente de Hermes: su personalidad y lo que puede usar se cambian en su perfil de Hermes; sus habilidades, aquí.",
    steps: [{ wait: "text=/funciona con Hermes/" }, { scroll: section("Agentes") }],
  }),
  settings({
    id: "ajustes-agente-editable",
    state: "Agentes · uno que se configura",
    description:
      "Claude Code, un agente ACP: personalidad editable, interruptores de acceso y sus skills.",
    steps: [
      { click: "role=button[name='Claude Code']" },
      { wait: "text=/se configura desde aquí/" },
      { scroll: section("Agentes") },
    ],
  }),
  settings({
    id: "ajustes-skill",
    state: "Agentes · editor de una skill",
    description: "El SKILL.md de una habilidad abierto para editarlo, con su nombre y su tamaño.",
    steps: [
      { click: "role=button[name='Claude Code']" },
      { click: "role=button[name='Editar revisar-pr']" },
      { wait: "role=textbox[name='Contenido del SKILL.md']" },
      { scroll: "role=textbox[name='Contenido del SKILL.md']" },
    ],
  }),
  settings({
    id: "ajustes-carpetas",
    state: "Carpetas que puede leer",
    description: "Las carpetas que el asistente puede leer; una no se puede abrir y dice por qué.",
    steps: [{ scroll: section("Carpetas que puede leer") }],
  }),
  settings({
    id: "ajustes-plantillas",
    state: "Plantillas de notas",
    description:
      "Las plantillas con las que se escriben las notas, con una propia abierta en el editor.",
    steps: [{ click: "role=button[name='Revisión de diseño']" }, { scroll: section("Reuniones") }],
  }),
  settings({
    id: "ajustes-diccionarios",
    state: "Diccionarios de reuniones",
    description:
      "Las palabras que la transcripción corrige, por diccionario, y las series que lo usan.",
    steps: [{ scroll: "text=Diccionarios de reuniones" }],
  }),
  settings({
    id: "ajustes-cuenta",
    state: "Cuenta y emparejamiento",
    description: "El nombre de la persona, el emparejamiento con el hub, el arranque y el aspecto.",
    steps: [{ scroll: section("Cuenta") }],
  }),
  settings({
    id: "ajustes-emparejar-de-nuevo",
    state: "Cuenta · emparejar de nuevo",
    description: "La advertencia antes de emparejar otra vez un Mac que ya lo está.",
    steps: [{ click: "role=button[name='Emparejar de nuevo']" }, { scroll: section("Cuenta") }],
  }),
  settings({
    id: "ajustes-actualizaciones",
    state: "Actualizaciones y privacidad",
    description:
      "La versión, buscar una actualización, «Parar todo ahora» y los informes de errores.",
    steps: [{ scroll: section("Actualizaciones") }],
  }),
  settings({
    id: "ajustes-permisos",
    state: "Permisos del sistema, falta uno",
    description:
      "Los permisos de macOS: tres concedidos y el de grabación de pantalla denegado, con cómo abrirlo.",
    views: (base) => ({ ...base, permissions: { ...base.permissions, screen: "denied" } }),
    steps: [{ scroll: section("Permisos del sistema") }],
  }),
];
