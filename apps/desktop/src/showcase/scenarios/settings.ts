import type { Scenario, Step, Views } from "../scenario";
import { privacyStopped } from "./edit";

/** Opens a section from Ajustes' index. */
const open = (label: string): Step => ({
  click: `role=navigation[name="Secciones de ajustes"] >> role=button[name=/^${label}/]`,
});
const region = (name: string) => `role=region[name="${name}"]`;

const settings = (
  scenario: Omit<Scenario, "view" | "route" | "ready"> & { ready?: string },
): Scenario => ({
  view: "Ajustes",
  route: "#/settings",
  ready: "role=heading[name='Ajustes']",
  ...scenario,
});

/** macOS has not given the screen and the system audio. */
const screenMissing = (base: Views): Views => ({
  ...base,
  permissions: { ...base.permissions, screen: "denied" },
});

/** Orbit busca versiones nuevas al abrirse: nothing searched by hand yet. */
const updatesIdle = (base: Views): Views => ({
  ...base,
  local: { ...base.local, update: { ...base.local.update, state: "idle" } },
});

/** The library before its first read from the hub. */
const libraryUnread = (base: Views): Views => {
  const notes = base.notes;
  if (notes?.library == null) return base;
  const unknown = { state: "unknown" as const, detail: null };
  return {
    ...base,
    notes: {
      ...notes,
      library: {
        ...notes.library,
        templates: unknown,
        links: unknown,
        dictionaries: unknown,
        items: [],
        dictionary_items: [],
      },
    },
  };
};

export const settingsScenarios: Scenario[] = [
  settings({
    id: "ajustes",
    state: "Privacidad y permisos, falta uno",
    description:
      "Parar todo, los cuatro permisos de macOS con el de pantalla sin dar y su botón, el micrófono siempre listo dentro de su fila y los informes de errores. El índice dice «● Falta 1».",
    views: screenMissing,
    steps: [open("Privacidad y permisos")],
  }),
  settings({
    id: "ajustes-todo-parado",
    state: "Privacidad y permisos, todo parado",
    description: "Tras «Parar todo»: «Todo está parado» con «Reactivar Orbit».",
    views: (base) => screenMissing(privacyStopped(base)),
    steps: [open("Privacidad y permisos"), { wait: "text=Todo está parado" }],
  }),
  settings({
    id: "ajustes-general",
    state: "General",
    description:
      "Tu nombre, este Mac y su hub, la bienvenida, el aspecto, abrir al iniciar sesión y las actualizaciones.",
    views: (base) => updatesIdle(screenMissing(base)),
  }),
  settings({
    id: "ajustes-emparejar-de-nuevo",
    state: "General · emparejar de nuevo",
    description: "La pregunta, dentro de la página, antes de emparejar otra vez un Mac emparejado.",
    views: updatesIdle,
    steps: [{ click: "role=button[name='Emparejar de nuevo']" }],
  }),
  settings({
    id: "ajustes-grabacion",
    state: "Grabación y llamadas",
    description:
      "Qué hace Orbit al empezar una videollamada, cuánto dura una grabación y los cuatro atajos.",
    steps: [open("Grabación y llamadas")],
  }),
  settings({
    id: "ajustes-asistente",
    state: "Asistente · Deimos, de Hermes",
    description:
      "Deimos funciona con Hermes: su personalidad, lo que puede usar y sus habilidades se ven tal como están.",
    steps: [open("Asistente"), { wait: "text=funciona con Hermes" }],
  }),
  settings({
    id: "ajustes-agente-editable",
    state: "Asistente · uno que se configura",
    description:
      "Claude Code se configura desde aquí: personalidad con su tamaño, interruptores y habilidades.",
    steps: [
      open("Asistente"),
      { click: "role=tab[name='Claude Code']" },
      { wait: "text=se configura desde aquí" },
    ],
  }),
  settings({
    id: "ajustes-nueva-habilidad",
    state: "Asistente · nueva habilidad",
    description: "«Nueva habilidad»: escribirla desde cero o importar una carpeta con su SKILL.md.",
    steps: [
      open("Asistente"),
      { click: "role=tab[name='Claude Code']" },
      { wait: "text=se configura desde aquí" },
      { scroll: region("Habilidades") },
      { click: "role=button[name='Nueva habilidad']" },
    ],
  }),
  settings({
    id: "ajustes-habilidad",
    state: "Asistente · editor de una habilidad",
    description: "El SKILL.md de una habilidad en su ventana, con su nombre y su tamaño.",
    steps: [
      open("Asistente"),
      { click: "role=tab[name='Claude Code']" },
      { click: "role=button[name='Editar revisar-pr']" },
      { wait: "role=textbox[name='Contenido del SKILL.md']" },
    ],
  }),
  settings({
    id: "ajustes-carpetas",
    state: "Asistente · carpetas, una con error",
    description:
      "Las carpetas que pueden leer los asistentes, «3 de 8»; macOS no deja abrir una y la fila dice qué hacer. Debajo, lo que ha hecho Deimos en este Mac.",
    steps: [
      open("Asistente"),
      { wait: "text=funciona con Hermes" },
      { scroll: region("Carpetas que puede leer") },
    ],
  }),
  settings({
    id: "ajustes-reuniones",
    state: "Reuniones · plantillas",
    description:
      "Las plantillas, una propia abierta en el editor que se guarda sola, con sus secciones «3 de 12».",
    steps: [open("Reuniones"), { click: "role=button[name='Revisión de diseño']" }],
  }),
  settings({
    id: "ajustes-diccionarios",
    state: "Reuniones · diccionarios",
    description:
      "Cada palabra que corrige la transcripción, «en lugar de…», con «Quitar», y cómo añadir otra.",
    steps: [open("Reuniones"), { scroll: region("Diccionarios") }],
  }),
  settings({
    id: "ajustes-cargando",
    state: "Reuniones · cargando",
    description:
      "Antes de leer la biblioteca del hub, las plantillas y los diccionarios se ven con su forma.",
    views: libraryUnread,
    steps: [open("Reuniones")],
  }),
];
