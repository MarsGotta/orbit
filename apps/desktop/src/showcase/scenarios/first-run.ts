import { waitingHub } from "../data/assistants";
import type { Scenario, Step } from "../scenario";
import { newMac, pairedMac } from "./first-run-macs";

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
  { click: "role=button[name='Emparejar después']" },
];
const toAssistants: Step[] = [...toPermissions, { click: "role=button[name='Continuar']" }];
/** A paired Mac goes on from «Tu hub» with «Continuar». */
const pairedToAssistants: Step[] = [
  { click: "role=button[name='Empezar']" },
  { click: "role=button[name='Continuar']" },
  { click: "role=button[name='Continuar']" },
];

export const firstRunScenarios: Scenario[] = [
  firstRun({
    id: "primera-vez-bienvenida",
    state: "1 · Bienvenida",
    description:
      "Paso 1 de 5: qué hace Orbit, los pasos que vienen, que todo se para con ⌃⌥Esc y «Empezar».",
  }),
  firstRun({
    id: "primera-vez-emparejar",
    state: "2 · Tu hub",
    description:
      "La dirección del hub, el código de 6 cifras (una vez, 5 minutos) y el nombre de este Mac, con «Emparejar después».",
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
    description:
      "El hub aceptó el código: «✓ Mac emparejado con …. Deimos ya está listo.» y «Continuar».",
    steps: [...toPair, pair, { wait: "text=✓ Mac emparejado con hub.nubaria.example" }],
  }),
  firstRun({
    id: "primera-vez-codigo-corto",
    state: "2 · No se pudo emparejar: el código",
    description:
      "Un código de 5 cifras no se manda: el campo en rojo dice cuántas tiene y el aviso qué comprobar.",
    steps: [
      { click: "role=button[name='Empezar']" },
      { fill: "role=textbox[name='Dirección del hub']", text: "https://hub.nubaria.example" },
      { fill: "role=textbox[name='Código de emparejamiento']", text: "48291" },
      { fill: "role=textbox[name='Nombre de este Mac']", text: "Mac de Lucía" },
      pair,
      { wait: "role=alert" },
    ],
  }),
  firstRun({
    id: "primera-vez-emparejar-fallo",
    state: "2 · No se pudo emparejar: el hub",
    description: "El hub no aceptó el emparejamiento: el aviso con su código y el paso sigue ahí.",
    refuse: (command) =>
      command.command === "pair" ? { code: "internal", retry: { kind: "never" } } : null,
    steps: [...toPair, pair, { wait: "role=alert" }],
  }),
  firstRun({
    id: "primera-vez-permisos",
    state: "3 · Permisos",
    description:
      "Los cuatro permisos: dos concedidos, accesibilidad con «Abrir Ajustes» y Calendarios «● Sin conceder» con «Abrir Ajustes del Sistema».",
    views: (base) =>
      newMac(base, { microphone: "granted", screen: "granted", calendars: "denied" }),
    steps: [...toPermissions, { wait: "role=button[name='Continuar']" }],
  }),
  firstRun({
    id: "primera-vez-permisos-esperando",
    state: "3 · Esperando a macOS",
    description:
      "El micrófono espera la respuesta de macOS; la pantalla ya está «✓ Concedido» y el calendario se pidió antes.",
    views: (base) => newMac(base, { screen: "granted", calendars: "denied" }),
    hold: (command) => command.command === "permissions",
    // The microphone's is the only «Pedir» left.
    steps: [
      ...toPermissions,
      { click: "role=button[name='Pedir']" },
      { wait: "text=Esperando a macOS…" },
    ],
  }),
  firstRun({
    id: "primera-vez-asistentes",
    state: "4 · Tus asistentes",
    description:
      "Deimos listo con el hub, Phobos conectado con su cuenta y Ares con «Iniciar sesión con Claude»; el principal, entre los que están listos.",
    views: pairedMac,
    steps: [...pairedToAssistants, { wait: "role=radiogroup[name='Asistente principal']" }],
  }),
  firstRun({
    id: "primera-vez-asistentes-iniciando",
    state: "4 · Iniciando sesión",
    description:
      "Ares espera al navegador: qué hacer allí, el código que devuelve la página de Claude y «Cancelar».",
    views: pairedMac,
    agent: waitingHub,
    steps: [
      ...pairedToAssistants,
      { click: "role=button[name='Iniciar sesión con Claude']" },
      { wait: "text=Esperando al navegador…" },
    ],
  }),
  firstRun({
    id: "primera-vez-asistentes-sin-hub",
    state: "4 · Sin hub",
    description:
      "Sin emparejar, Deimos «● Necesita tu hub» con «Emparejar el hub», que vuelve al paso 2; aún no hay principal que elegir.",
    steps: [...toAssistants, { wait: "role=button[name='Emparejar el hub']" }],
  }),
  firstRun({
    id: "primera-vez-listo",
    state: "5 · Todo listo",
    description:
      "El resumen con su punto verde o ámbar (el hub, «3 de 4 concedidos · falta Calendarios», «Deimos (principal) y Phobos») y los tres atajos.",
    views: pairedMac,
    steps: [
      ...pairedToAssistants,
      { click: "role=button[name='Continuar']" },
      { wait: "role=button[name='Empezar a usar Orbit']" },
    ],
  }),
  firstRun({
    id: "primera-vez-listo-falta",
    state: "5 · Todo listo, falta todo",
    description: "Saltado todo: el resumen dice en ámbar lo que falta y dónde hacerlo después.",
    steps: [
      ...toAssistants,
      { click: "role=button[name='Continuar']" },
      { wait: "role=button[name='Empezar a usar Orbit']" },
    ],
  }),
];
