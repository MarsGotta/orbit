import { macPermissions } from "./permissions";
import { globalShortcuts } from "./shortcuts";

const base = {
  stepOf: (step: number, total: number) => `Paso ${step} de ${total}`,
  stepsLabel: "Pasos",
  stepLabel: (step: number, name: string) => `Paso ${step}: ${name}`,
  stepNames: ["Bienvenida", "Tu hub", "Permisos", "Listo"],
  welcome: "Te damos la bienvenida a Orbit",
  welcomeBody:
    "En un par de minutos lo dejas listo: conectas este Mac con tu hub y eliges qué puede usar Orbit.",
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
      title: "Un asistente que te echa una mano",
      body: "Pregúntale, llámale o pídele que haga cosas en tu Mac. Antes de actuar, te pide permiso.",
    },
  ],
  welcomeControl: `Tú decides qué ve y oye Orbit. Lo paras todo al instante con ${globalShortcuts.stop}.`,
  begin: "Empezar",
  continue: "Continuar",
  back: "Atrás",
  pair: "Emparejar",
  pairTitle: "Conecta este Mac con tu hub",
  pairBody:
    "El hub es donde se guardan tus reuniones y notas. Quien lo gestiona te da la dirección y un código.",
  pairing: "Emparejando…",
  paired: (host: string) => `✓ Mac emparejado con ${host}`,
  hubUrl: "Dirección del hub",
  hubUrlPlaceholder: "https://hub.example",
  code: "Código de emparejamiento",
  codeHelp: "Sirve una sola vez y caduca a los 5 minutos.",
  deviceName: "Nombre de este Mac",
  permissions: "Elige qué puede usar Orbit",
  permissionsBody: "macOS te preguntará por cada uno. Puedes cambiarlo cuando quieras en Ajustes.",
  permissionsLater: "Puedes seguir sin alguno: te diremos qué falta justo cuando lo necesites.",
  allow: "Permitir",
  allowed: "✓ Permitido",
  openSettings: "Abrir Ajustes",
  accessibility: macPermissions.accessibility.label,
  ready: "Todo listo",
  readyBody: "Tres atajos que funcionan desde cualquier app:",
  // «Configurarlo más tarde» skips the whole first run; «Hacerlo después», one step.
  skipAll: "Configurarlo más tarde",
  skipStep: "Hacerlo después",
  start: "Empezar a usar Orbit",
} as const;

/** The first run: four steps, each saying where it is and what each skip leaves. */
export function onboardingCopy(assistantName: string) {
  return {
    onboarding: {
      ...base,
      // Why each permission, in the order of the rows.
      permissionWhy: {
        microphone: `Para grabar reuniones, mandar notas de voz y hablar con ${assistantName}.`,
        screen: `Para grabar el sonido de tus videollamadas y que ${assistantName} vea la pantalla si se lo pides.`,
        accessibility: `Para que ${assistantName} pueda usar tu Mac y leer el texto que selecciones.`,
        calendars: "Para ver y cambiar tu agenda desde Orbit.",
      },
      shortcuts: [
        {
          keys: globalShortcuts.meeting,
          title: "Grabar una reunión",
          body: "Y pulsarlo otra vez para terminar.",
        },
        {
          keys: globalShortcuts.ptt,
          title: `Hablar con ${assistantName}`,
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
