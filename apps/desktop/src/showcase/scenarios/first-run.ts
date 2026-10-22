import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import type { Scenario, Step, Views } from "../scenario";

/** A Mac that has just installed Orbit: not paired, no permission asked yet. */
function newMac(base: Views, granted: Partial<PermissionSnapshot> = {}): Views {
  const unasked = "not_determined" as const;
  const permissions = {
    ...base.permissions,
    microphone: unasked,
    screen: unasked,
    // macOS never asks for accessibility: it is switched on in System Settings.
    accessibility: "not_granted" as const,
    calendars: unasked,
    ...granted,
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
  scenario: Pick<Scenario, "id" | "state" | "description"> & Partial<Scenario>,
): Scenario => ({
  view: "Primer arranque",
  route: "#/briefing",
  views: (base) => newMac(base),
  onboarded: false,
  // What is on screen before the steps: the welcome.
  ready: "role=button[name='Empezar']",
  ...scenario,
});

const toPair: Step[] = [
  { click: "role=button[name='Empezar']" },
  { fill: "role=textbox[name='Dirección del hub']", text: "https://hub.nubaria.example" },
  { fill: "role=textbox[name='Código de emparejamiento']", text: "482913" },
  { fill: "role=textbox[name='Nombre de este Mac']", text: "Mac de Lucía" },
];
const pair: Step = { click: "role=button[name='Emparejar']" };
const toPermissions: Step[] = [
  { click: "role=button[name='Empezar']" },
  { click: "role=button[name='Hacerlo después']" },
];

export const firstRunScenarios: Scenario[] = [
  firstRun({
    id: "primera-vez-bienvenida",
    state: "1 · Bienvenida",
    description: "Paso 1 de 4: qué hace Orbit, que todo se para con ⌃⌥Esc y «Empezar».",
  }),
  firstRun({
    id: "primera-vez-emparejar",
    state: "2 · Tu hub",
    description:
      "La dirección del hub, el código de 6 cifras (una vez, 5 minutos) y el nombre de este Mac.",
    steps: [...toPair, { wait: "role=button[name='Emparejar']" }],
  }),
  firstRun({
    id: "primera-vez-emparejando",
    state: "2 · Emparejando",
    description: "«Emparejar» dice «Emparejando…» mientras el hub contesta.",
    hold: (command) => command.command === "pair",
    steps: [...toPair, pair, { wait: "role=button[name='Emparejando…']" }],
  }),
  firstRun({
    id: "primera-vez-emparejado",
    state: "2 · Emparejado",
    description: "El hub aceptó el código: «✓ Mac emparejado con …» y «Continuar».",
    steps: [...toPair, pair, { wait: "text=✓ Mac emparejado con hub.nubaria.example" }],
  }),
  firstRun({
    id: "primera-vez-emparejar-fallo",
    state: "2 · No se pudo emparejar",
    description: "El hub no aceptó el emparejamiento: el fallo con su código y el paso sigue ahí.",
    refuse: (command) =>
      command.command === "pair" ? { code: "internal", retry: { kind: "never" } } : null,
    steps: [...toPair, pair, { wait: "role=alert" }],
  }),
  firstRun({
    id: "primera-vez-permisos",
    state: "3 · Permisos",
    description: "Los cuatro permisos, cada uno con para qué sirve y «Permitir» o «Abrir Ajustes».",
    steps: [...toPermissions, { wait: "role=button[name='Continuar']" }],
  }),
  firstRun({
    id: "primera-vez-permisos-esperando",
    state: "3 · Esperando a macOS",
    description:
      "El micrófono espera la respuesta de macOS; la pantalla ya está «✓ Permitido» y el calendario se pidió antes.",
    views: (base) => newMac(base, { screen: "granted", calendars: "denied" }),
    hold: (command) => command.command === "permissions",
    // The microphone's is the only «Permitir» left.
    steps: [
      ...toPermissions,
      { click: "role=button[name='Permitir']" },
      { wait: "text=Esperando a macOS…" },
    ],
  }),
  firstRun({
    id: "primera-vez-listo",
    state: "4 · Todo listo",
    description: "Los tres atajos y «Empezar a usar Orbit».",
    steps: [
      ...toPermissions,
      { click: "role=button[name='Continuar']" },
      { wait: "role=button[name='Empezar a usar Orbit']" },
    ],
  }),
];
