import type { Scenario, Views } from "../scenario";
import { privacyStopped, withStatus } from "./edit";

// The notices of the top bar, one at a time with the button that fixes it,
// and what the whole window says before Orbit answers or when it stops.

const notice = (
  id: string,
  state: string,
  description: string,
  views: (base: Views) => Views,
  ready: string,
): Scenario => ({
  id,
  view: "Barra superior",
  state,
  description,
  route: "#/briefing",
  views,
  ready: `role=status >> text=${ready}`,
});

const missingMicrophone = (base: Views): Views => ({
  ...base,
  permissions: { ...base.permissions, microphone: "denied" },
});

export const windowNoticeScenarios: Scenario[] = [
  notice(
    "aviso-barra-sin-conexion",
    "Sin conexión con el hub",
    "El hub no responde: el aviso lo dice y que Orbit sigue intentándolo.",
    (base) =>
      withStatus(
        base,
        { connection: { daemon: "ready", hub: "offline" } },
        { connection: "offline" },
      ),
    "Sin conexión con el hub",
  ),
  notice(
    "aviso-barra-todo-parado",
    "Todo está parado",
    "Tras «Parar todo», el aviso ofrece «Reactivar».",
    privacyStopped,
    "Todo está parado",
  ),
  notice(
    "aviso-barra-sin-emparejar",
    "Mac sin emparejar",
    "Este Mac no está emparejado con ningún hub: «Emparejar» abre el emparejamiento.",
    (base) =>
      withStatus(
        base,
        { connection: { daemon: "ready", hub: "unpaired" } },
        { paired: false, connection: "offline" },
      ),
    "Este Mac no está emparejado",
  ),
  notice(
    "aviso-barra-retirado",
    "Mac retirado",
    "El hub retiró este Mac: «Emparejar» para volver a conectarlo.",
    (base) => withStatus(base, {}, { connection: "revoked" }),
    "El hub retiró este Mac",
  ),
  notice(
    "aviso-barra-permiso",
    "Falta un permiso",
    "macOS no ha dado un permiso: «Arreglar» lleva a Ajustes › Privacidad y permisos.",
    missingMicrophone,
    "Falta el permiso de",
  ),
  {
    ...notice(
      "aviso-barra-varios",
      "Varios avisos",
      "Dos cosas a la vez: se ve la más importante y, al pulsar «+1», el resto con su arreglo.",
      (base) => missingMicrophone(privacyStopped(base)),
      "Todo está parado",
    ),
    steps: [
      { click: "role=button[name='Ver 1 aviso más']" },
      { wait: "role=dialog[name='Avisos']" },
    ],
  },
  notice(
    "aviso-barra-conectando",
    "Conectando con el hub",
    "Al abrir, mientras conecta con el hub por primera vez: lo dice en lugar de callar.",
    (base) =>
      withStatus(
        base,
        { connection: { daemon: "ready", hub: "connecting" } },
        { connection: "connecting" },
      ),
    "Conectando con tu hub…",
  ),
  notice(
    "aviso-barra-preparando",
    "Orbit se prepara",
    "Justo después de abrir, mientras Orbit recupera su estado.",
    (base) => withStatus(base, { connection: { daemon: "connecting", hub: "paired" } }),
    "Orbit se está preparando…",
  ),
  {
    id: "ventana-conectando",
    view: "Al abrir Orbit",
    state: "Conectando",
    description: "Antes del primer estado de Orbit, con lo que pasará si tarda.",
    route: "#/briefing",
    connect: "hang",
    ready: "text=Si tarda más de 15 segundos, te diremos qué pasa.",
  },
  {
    id: "ventana-tardando",
    view: "Al abrir Orbit",
    state: "Tarda demasiado",
    description:
      "A los 15 s sin el estado de Orbit: qué pasa, que lo guardado está a salvo y «Cerrar Orbit».",
    route: "#/briefing",
    connect: "hang",
    ready: "text=Conectando con Orbit…",
    steps: [{ wait: "role=button[name='Cerrar Orbit']" }],
  },
  {
    id: "ventana-no-responde",
    view: "Orbit no responde",
    state: "No conecta",
    description: "La ventana no puede leer el estado de Orbit: qué hacer y «Cerrar Orbit».",
    route: "#/briefing",
    connect: "fail",
    ready: "text=Orbit no responde",
  },
  {
    id: "ventana-orbit-perdido",
    view: "Orbit no responde",
    state: "Dejó de responder",
    description: "La parte de Orbit que graba y guarda dejó de responder con la ventana abierta.",
    route: "#/briefing",
    views: (base) => withStatus(base, { connection: { daemon: "lost", hub: "paired" } }),
    ready: "text=Orbit no responde",
  },
];
