import type { Scenario, Views } from "../scenario";

/** A Mac that has just installed Orbit: not paired, no permission asked yet. */
function newMac(base: Views): Views {
  const unasked = "not_determined" as const;
  const permissions = {
    ...base.permissions,
    microphone: unasked,
    screen: unasked,
    accessibility: unasked,
    calendars: unasked,
  };
  return {
    ...base,
    permissions,
    status: {
      ...base.status,
      connection: { daemon: "ready", hub: "unpaired" },
      settings: {
        ...base.status.settings,
        hub: {
          url: null,
          paired: false,
          device_name: null,
          device_id_short: null,
          connection: "offline",
        },
      },
    },
    local: { ...base.local, onboarding_done: false, profile: null },
    agenda: { ...base.agenda, authorization: "not_determined", events: [] },
    notes: { list: [], open: null, index: { state: "needs_hub", more: false, detail: null } },
    briefing: null,
  };
}

const firstRun = (
  scenario: Pick<Scenario, "id" | "state" | "description" | "steps">,
): Scenario => ({
  view: "Primera vez",
  route: "#/briefing",
  views: newMac,
  onboarded: false,
  ready: "role=button[name='Continuar']",
  ...scenario,
});

export const firstRunScenarios: Scenario[] = [
  firstRun({
    id: "primera-vez-bienvenida",
    state: "Bienvenida",
    description: "El primer arranque: qué es Orbit y los pasos que vienen.",
  }),
  firstRun({
    id: "primera-vez-emparejar",
    state: "Emparejar con el hub",
    description: "La dirección del hub, el código de emparejamiento y el nombre de este Mac.",
    steps: [
      { click: "role=button[name='Continuar']" },
      { fill: "role=textbox[name='URL del hub']", text: "https://hub.example" },
      { fill: "role=textbox[name='Código']", text: "482-913" },
      { fill: "role=textbox[name='Nombre del dispositivo']", text: "Mac de Lucía" },
    ],
  }),
  firstRun({
    id: "primera-vez-permisos",
    state: "Permisos de macOS",
    description: "Los permisos que Orbit pide a macOS, cada uno con lo que permite y su botón.",
    steps: [
      { click: "role=button[name='Continuar']" },
      { click: "role=button[name='Saltar este paso']" },
      { wait: "role=button[name='Continuar']" },
    ],
  }),
  firstRun({
    id: "primera-vez-listo",
    state: "Listo",
    description: "El último paso del primer arranque.",
    steps: [
      { click: "role=button[name='Continuar']" },
      { click: "role=button[name='Saltar este paso']" },
      { click: "role=button[name='Continuar']" },
      { wait: "role=button[name='Empezar a usar Orbit']" },
    ],
  }),
];
