import { listOf } from "./notes";
import { macPermissions } from "./permissions";
import { globalShortcuts } from "./shortcuts";

const base = {
  stepOf: (step: number, total: number, name: string) => `Paso ${step} de ${total} · ${name}`,
  stepsLabel: "Pasos",
  stepLabel: (step: number, name: string) => `Paso ${step}: ${name}`,
  stepNames: ["Bienvenida", "Tu hub", "Permisos", "Asistentes", "Listo"],
  welcome: "Te damos la bienvenida a Orbit",
  welcomeBody:
    "En un par de minutos lo dejas listo: conectas este Mac con tu hub, eliges qué puede usar Orbit y con qué asistentes vas a hablar.",
  welcomePoints: [
    {
      title: "Graba tus reuniones y olvídate de tomar notas",
      body: "Orbit escribe el resumen, las tareas de cada persona y la transcripción.",
    },
    {
      title: "Tu día de un vistazo",
      body: "Con tu calendario de siempre y lo que quedó pendiente de cada reunión.",
    },
    {
      title: "Asistentes que te echan una mano",
      body: "Pregúntales, llámales o pídeles que hagan cosas en tu Mac. Antes de actuar, te piden permiso.",
    },
  ],
  planLabel: "Los pasos que vienen",
  welcomeControl: `Tú decides qué ve y oye Orbit. Lo paras todo al instante con ${globalShortcuts.stop}.`,
  begin: "Empezar",
  continue: "Continuar",
  back: "Atrás",
  pair: "Emparejar",
  pairTitle: "Conecta este Mac con tu hub",
  pairing: "Emparejando…",
  hubUrl: "Dirección del hub",
  hubUrlPlaceholder: "https://hub.example",
  code: "Código de emparejamiento",
  codeHelp: "Sirve una sola vez y caduca a los 5 minutos.",
  codeDigits: (count: number) => `Tiene ${count} ${count === 1 ? "cifra" : "cifras"}; deben ser 6.`,
  deviceName: "Nombre de este Mac",
  pairFailed: "No se pudo emparejar",
  pairFailedHint:
    "El código tiene 6 cifras y sirve una sola vez. Comprueba también la dirección del hub o pide un código nuevo a quien lo gestiona.",
  permissions: "Elige qué puede usar Orbit",
  permissionsBody: "macOS te preguntará por cada uno. Puedes cambiarlo cuando quieras en Ajustes.",
  permissionsLater: "Puedes seguir sin alguno: Orbit te dirá qué falta justo cuando lo necesites.",
  allow: "Pedir",
  allowed: "✓ Concedido",
  notGranted: "Sin conceder",
  openSettings: "Abrir Ajustes",
  openSystemSettings: "Abrir Ajustes del Sistema",
  waitingForSettings: "Esperando a Ajustes del Sistema…",
  accessibility: macPermissions.accessibility.label,
  assistants: "Tus asistentes",
  assistantsBody: (hub: string) =>
    `Cada uno funciona con un servicio distinto. ${hub} viene con tu hub; para los demás, inicia sesión con tu suscripción.`,
  readyWithHub: "✓ Listo con tu hub",
  connected: "✓ Conectado",
  needsHub: "Necesita tu hub",
  pairHub: "Emparejar el hub",
  signInWith: (vendor: string) => `Iniciar sesión con ${vendor}`,
  waitingForBrowser: "Esperando al navegador…",
  account: (email: string | undefined, plan: string | undefined) =>
    [email, plan].filter((part) => part !== undefined && part !== "").join(" · "),
  primary: "Asistente principal",
  primaryHelp: (other: string | undefined) =>
    other === undefined
      ? "Responde a «Oye…» sin nombre y en el panel flotante."
      : `Responde a «Oye…» sin nombre y en el panel flotante. A los demás les llamas por su nombre: «Oye ${other}, …».`,
  noPrimary: "Conecta al menos uno para elegir el principal.",
  assistantsLater:
    "Puedes seguir sin alguno. Añades más o cambias sus cuentas cuando quieras, desde Asistente.",
  ready: "Todo listo",
  readyBody: "Orbit queda preparado. Puedes cambiar cualquier cosa desde Ajustes.",
  summaryLabel: "Lo que has configurado",
  summaryHub: "Tu hub",
  summaryPermissions: "Permisos",
  summaryAssistants: "Asistentes",
  hubPaired: (host: string, device: string | null) =>
    device === null || device === "" ? host : `${host} · este Mac se llama «${device}»`,
  hubUnpaired: "Sin emparejar. Hazlo cuando quieras en Ajustes › General.",
  allGranted: "Los cuatro concedidos",
  someGranted: (granted: number, missing: string[]) =>
    `${granted} de 4 concedidos · falta ${listOf(missing)}`,
  assistantsReady: (names: string[]) => listOf(names),
  asPrimary: (name: string) => `${name} (principal)`,
  noAssistants: "Ninguno conectado todavía",
  shortcutsTitle: "Tres atajos que funcionan desde cualquier app",
  // «Configurarlo más tarde» skips the whole first run; «Emparejar después», pairing.
  skipAll: "Configurarlo más tarde",
  skipStep: "Emparejar después",
  start: "Empezar a usar Orbit",
} as const;

/** The first run: five steps, each saying where it is and what each skip leaves. */
export function onboardingCopy(assistantName: string) {
  return {
    onboarding: {
      ...base,
      /** The steps to come, on the welcome; `others` are the assistants with a subscription. */
      plan: (hub: string, others: string[]) => [
        { title: "Tu hub", body: "Conecta este Mac con el hub donde se guardan tus reuniones." },
        { title: "Permisos", body: "Micrófono, pantalla, accesibilidad y calendarios." },
        {
          title: "Asistentes",
          body:
            others.length === 0
              ? `${hub} viene con tu hub; los demás, con tu suscripción.`
              : `${hub} viene con tu hub; ${listOf(others)}, con tu suscripción.`,
        },
        { title: "Listo", body: "Tres atajos para empezar." },
      ],
      pairBody: (hub: string) =>
        `El hub es donde se guardan tus reuniones y notas, y donde vive ${hub}. Quien lo gestiona te da la dirección y un código.`,
      paired: (host: string, hub: string) => `✓ Mac emparejado con ${host}. ${hub} ya está listo.`,
      // Why each permission, in the order of the rows.
      permissionWhy: {
        microphone: "Para grabar reuniones, mandar notas de voz y hablar con tus asistentes.",
        screen:
          "Para grabar el sonido de tus videollamadas y que tus asistentes vean la pantalla si se lo pides.",
        accessibility:
          "Para que tus asistentes puedan usar el Mac y leer el texto que selecciones.",
        calendars: "Para ver y cambiar tu agenda desde Orbit.",
      },
      shortcuts: (primary: string = assistantName) => [
        {
          keys: globalShortcuts.meeting,
          title: "Grabar una reunión",
          body: "Y pulsarlo otra vez para terminar.",
        },
        {
          keys: globalShortcuts.ptt,
          title: `Hablar con ${primary}`,
          body: "Mantenlo pulsado, habla y suelta.",
        },
        {
          keys: globalShortcuts.stop,
          title: "Pararlo todo al instante",
          body: "Micrófono, pantalla y subidas, hasta que lo reactives.",
        },
      ],
    },
  } as const;
}
