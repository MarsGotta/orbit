import { hourOf, whenBack } from "./assistantsFormat";

// Signing an assistant in from Orbit and its plan's limit: what the person
// does in the browser is «iniciar sesión»; tokens and OAuth stay out.
export const assistantsAccountCopy = {
  // Signing in.
  signIn: (vendor: string) => `Iniciar sesión en ${vendor}`,
  signInAgainTitle: (vendor: string) => `Inicia sesión otra vez en ${vendor}`,
  signInFirstTitle: (vendor: string) => `Inicia sesión en ${vendor}`,
  signInAgainText: (name: string) =>
    `Tus conversaciones con ${name} siguen aquí; solo falta tu sesión para que pueda responder.`,
  signInFirstText: (name: string) =>
    `${name} responde con tu plan cuando inicies sesión. Lo que escribas se queda aquí.`,
  waitingTitle: "Esperando a que inicies sesión en el navegador…",
  waitingText: (vendor: string) =>
    `Se ha abierto tu navegador. Inicia sesión con tu cuenta de ${vendor} y vuelve aquí.`,
  deviceCode: (code: string) => `Escribe este código en la página: ${code}`,
  pasteCode: "Código de la página",
  pasteCodeHelp: (vendor: string) =>
    `Al terminar, la página de ${vendor} te da un código. Pégalo aquí.`,
  sendCode: "Enviar código",
  cancelSignIn: "Cancelar",
  connected: "✓ Conectado",
  connectedAs: (email: string | undefined, plan: string | undefined) =>
    email === undefined
      ? plan === undefined
        ? "Con tu cuenta"
        : `Con tu cuenta · ${plan}`
      : plan === undefined
        ? `Como ${email}`
        : `Como ${email} · ${plan}`,
  sameAccount: (other: string) =>
    `Esta cuenta ya la usa ${other}. Puedes seguir: los dos gastan del mismo plan.`,
  browserNotOpened: "No se pudo abrir el navegador. Copia la dirección y ábrela tú:",
  /** A sign-in that ended without one, by the hub's cause. */
  signInEnded: (cause: string | undefined, vendor: string, name: string): string => {
    switch (cause) {
      case "cancelled":
        return `Cancelaste el inicio de sesión. ${name} sigue como estaba.`;
      case "device_code_disabled":
        return `Tu cuenta de ${vendor} no permite iniciar sesión con un código. Actívalo en la seguridad de tu cuenta y vuelve a intentarlo.`;
      case "plan_required":
        return vendor === "Claude"
          ? "Esta cuenta no tiene el plan que pide Claude Code. Inicia sesión con una cuenta de Claude Pro, Max, Team o Enterprise."
          : "Esta cuenta no tiene el plan que pide Codex. Inicia sesión con una cuenta de ChatGPT Plus, Pro, Business, Edu o Enterprise.";
      case "expired":
        return `El código de ${vendor} caducó antes de que terminaras. Vuelve a intentarlo.`;
      case "code_rejected":
        return `${vendor} no aceptó el código. Copia el código entero de la página y vuelve a intentarlo.`;
      case "login_disabled":
        return `La configuración de ${vendor} en tu hub no permite iniciar sesión así. Pídeselo a quien gestiona tu hub.`;
      case "timed_out":
        return "Pasaron 15 minutos sin terminar el inicio de sesión. Vuelve a intentarlo.";
      case "no_url":
        return `La herramienta de ${vendor} no dio una dirección para iniciar sesión en 30 segundos. Vuelve a intentarlo; si se repite, avisa a quien gestiona tu hub.`;
      case "no_credentials":
        return `La herramienta de ${vendor} terminó sin guardar tu sesión. Vuelve a intentarlo.`;
      case "not_started":
        return `La herramienta de ${vendor} no se pudo abrir en tu hub. Avisa a quien gestiona tu hub.`;
      case "exited":
        return `La herramienta de ${vendor} terminó con un error. Vuelve a intentarlo.`;
      default:
        return `El inicio de sesión no terminó (${cause ?? "sin causa"}). Vuelve a intentarlo.`;
    }
  },
  tryAgain: "Volver a intentarlo",
  composerSignIn: (name: string, vendor: string) =>
    `${name} no puede responder hasta que inicies sesión en ${vendor}. Lo que escribas se queda aquí.`,
  composerLimit: (name: string, atMs: number | undefined) =>
    atMs === undefined
      ? `${name} no puede responder hasta que se renueve tu plan. Lo que escribas se queda aquí.`
      : `${name} no puede responder hasta las ${hourOf(atMs)}. Lo que escribas se queda aquí.`,

  // The plan's limit.
  limitTitle: (name: string, vendor: string) =>
    `${name} ha llegado al límite de tu plan de ${vendor}`,
  limitTitlePlan: (name: string) => `${name} ha llegado al límite de su plan`,
  limitText: (atMs: number | undefined, nowMs: number) =>
    `${atMs === undefined ? "Vuelve a estar disponible cuando se renueve tu plan." : `Vuelve a estar disponible ${whenBack(atMs, nowMs)}.`} Tu pregunta no se ha perdido: puedes seguir con otro asistente, que leerá lo que habéis hablado.`,
  continueWith: (name: string) => `Seguir con ${name}`,
  signInFirst: (name: string) => `${name} · inicia sesión primero`,
  signInFirstTip: (vendor: string) => `Antes tienes que iniciar sesión en ${vendor}`,
  waitUntil: (atMs: number) => `Esperar a las ${hourOf(atMs)}`,
  whyPrimary: (name: string) => `Te proponemos a ${name} porque es tu principal.`,
  handedOver: (name: string) => `Sigues con ${name} · le hemos pasado esta conversación`,
  waitingFor: (name: string, atMs: number) =>
    `Te avisamos cuando ${name} vuelva a estar disponible (${hourOf(atMs)}). Mandaremos tu pregunta entonces.`,
  continueWithOther: "Seguir con otro",
} as const;
