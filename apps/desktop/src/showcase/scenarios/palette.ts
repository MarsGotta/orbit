import type { Scenario } from "../scenario";

const open = { press: "Meta+Shift+Space" } as const;

const palette = (
  scenario: Pick<Scenario, "id" | "state" | "description"> & Partial<Scenario>,
): Scenario => ({
  view: "Buscar y órdenes",
  route: "#/briefing",
  ready: "text=Pendiente de la última vez",
  ...scenario,
});

/** The palette (⌘⇧␣) and the generic failure with its two actions. */
export const paletteScenarios: Scenario[] = [
  palette({
    id: "paleta",
    state: "Abierta",
    description:
      "Acciones, Ir a y Privacidad: cada fila con su icono, su subtítulo y su atajo; ↑↓ ↵ esc abajo.",
    steps: [open, { wait: "role=listbox" }],
  }),
  palette({
    id: "paleta-filtrada",
    state: "Filtrada",
    description: "«reunion» filtra sin tildes ni mayúsculas; preguntar a Deimos queda al final.",
    steps: [open, { fill: "role=combobox", text: "reunion" }, { wait: "text=Reuniones" }],
  }),
  palette({
    id: "paleta-preguntar",
    state: "Preguntar a Deimos",
    description: "Nada se llama «ribera»: la primera fila lo pregunta a Deimos y ↵ lo manda.",
    steps: [
      open,
      { fill: "role=combobox", text: "ribera" },
      { wait: "text=Ninguna orden se llama así. Pulsa ↵ y se lo preguntamos a Deimos." },
    ],
  }),
  {
    id: "fallo-aviso",
    view: "Avisos",
    state: "Fallo con «Reintentar»",
    description:
      "Un fallo en la esquina: qué no se pudo, qué hacer, el código, «Reintentar» y «Copiar el código».",
    route: "#/briefing",
    refuse: (command) =>
      command.command === "privacy_stop" ? { code: "busy", retry: { kind: "after_fix" } } : null,
    ready: "text=Pendiente de la última vez",
    steps: [
      open,
      { fill: "role=combobox", text: "parar todo" },
      { press: "Enter" },
      { wait: "text=No se pudo parar todo" },
    ],
  },
  {
    id: "fallo-en-la-vista",
    view: "Avisos",
    state: "Fallo dentro de la vista",
    description:
      "La llamada no pudo empezar: el fallo sale en el Asistente con «Reintentar» y «Copiar el código».",
    route: "#/briefing",
    refuse: (command) =>
      command.command === "call_start"
        ? { code: "busy", retry_after_ms: 2000, retry: { kind: "after_ms", ms: 2000 } }
        : null,
    ready: "text=Pendiente de la última vez",
    steps: [
      open,
      { fill: "role=combobox", text: "llamar" },
      { press: "Enter" },
      { wait: "text=No se pudo empezar la llamada" },
    ],
  },
];
