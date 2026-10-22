import type { Scenario } from "../scenario";
import { at } from "../data/time";
import { notices, privacyStopped, withStatus } from "./edit";

const pill = (
  id: string,
  state: string,
  description: string,
  views: Scenario["views"],
  ready: string,
): Scenario => ({
  id,
  view: "Barra superior",
  state,
  description,
  route: "#/briefing",
  views,
  ready,
});

export const shellScenarios: Scenario[] = [
  {
    id: "hoy",
    view: "Hoy",
    state: "Un día normal",
    description:
      "El resumen del día: saludo, la próxima reunión con lo pendiente de la anterior y la agenda.",
    route: "#/briefing",
    ready: "text=A tratar",
  },
  {
    id: "hoy-sin-reuniones",
    view: "Hoy",
    state: "Sin reuniones",
    description: "Un día sin reuniones en la agenda: el resumen lo dice y ofrece grabar.",
    route: "#/briefing",
    views: (base) => ({
      ...base,
      briefing: base.briefing && {
        ...base.briefing,
        summary: { meetings_today: 0, focus_minutes: 480 },
        next: null,
        upcoming: [],
        agenda: [],
      },
    }),
    ready: "text=Agenda de hoy",
  },
  {
    id: "paleta",
    view: "Paleta de órdenes",
    state: "Abierta",
    description:
      "«Pregunta a Orbit» (⌘⇧Espacio): ir a una vista, grabar, preguntar o parar por privacidad.",
    route: "#/briefing",
    ready: "text=A tratar",
    steps: [{ click: "role=button[name=/Pregunta a Orbit/]" }, { wait: "role=dialog" }],
  },
  {
    id: "cargando",
    view: "Arranque",
    state: "Conectando",
    description: "La ventana antes de que Orbit mande su primer estado.",
    route: "#/briefing",
    connect: "hang",
    ready: "text=Conectando con Orbit…",
  },
  {
    id: "sin-conexion-con-orbit",
    view: "Arranque",
    state: "No conecta",
    description: "La ventana cuando no puede leer el estado de Orbit.",
    route: "#/briefing",
    connect: "fail",
    ready: "text=No se pudo conectar con Orbit",
  },
  pill(
    "pildora-sin-red",
    "Píldora · sin conexión",
    "La píldora de estado cuando el hub no responde.",
    (base) =>
      withStatus(
        base,
        { connection: { daemon: "ready", hub: "offline" } },
        { connection: "offline" },
      ),
    "text=Sin conexión",
  ),
  pill(
    "pildora-sin-emparejar",
    "Píldora · sin emparejar",
    "La píldora cuando este Mac no está emparejado con ningún hub, con su botón.",
    (base) =>
      withStatus(
        base,
        { connection: { daemon: "ready", hub: "unpaired" } },
        { paired: false, connection: "offline" },
      ),
    "text=Este Mac no está emparejado",
  ),
  pill(
    "pildora-retirado",
    "Píldora · Mac retirado",
    "La píldora cuando el hub retiró este Mac.",
    (base) => withStatus(base, {}, { connection: "revoked" }),
    "text=El hub retiró este Mac",
  ),
  pill(
    "pildora-permiso",
    "Píldora · falta un permiso",
    "La píldora cuando macOS no ha dado un permiso, con el camino a Ajustes.",
    (base) => ({ ...base, permissions: { ...base.permissions, microphone: "denied" } }),
    "text=Falta el permiso",
  ),
  pill(
    "pildora-orbit-no-responde",
    "Píldora · Orbit no responde",
    "La píldora cuando la parte de Orbit que graba y guarda dejó de responder.",
    (base) => withStatus(base, { connection: { daemon: "lost", hub: "paired" } }),
    "text=Orbit no responde",
  ),
  {
    id: "privacidad-detenida",
    view: "Privacidad",
    state: "Todo parado",
    description: "Tras «Parar todo»: la píldora ofrece rearmar y la agenda dice por qué no se lee.",
    route: "#/calendar",
    views: privacyStopped,
    ready: "text=Privacidad detenida",
  },
  {
    id: "aviso-actualizacion",
    view: "Avisos",
    state: "Actualización disponible",
    description: "El aviso de una versión nueva, con su botón para instalarla.",
    route: "#/briefing",
    views: (base) => ({
      ...base,
      local: {
        ...base.local,
        update: { ...base.local.update, state: "available", version: "0.11.0" },
      },
    }),
    ready: "text=Hay una versión nueva de Orbit (0.11.0)",
  },
  {
    id: "avisos",
    view: "Avisos",
    state: "Varios a la vez",
    description:
      "Los avisos apilados: uno hecho, uno que se puede arreglar y un fallo con su código.",
    route: "#/briefing",
    later: (views) =>
      notices(
        views,
        { code: "recording_finished", data: { recording: "rec-daily-21", at_ms: at(0, 9, 14) } },
        { code: "meeting_audio_microphone_only" },
        { code: "contained_failure", data: { step: "upload_jobs" } },
      ),
    ready: "text=Grabación guardada.",
  },
];
