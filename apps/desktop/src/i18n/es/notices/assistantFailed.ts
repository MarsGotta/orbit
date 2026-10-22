import { ATTACHMENT_NOT_FOUND, fileSize, sessionCeilingText, tooManyText } from "../attachments";
import type { Entries, NoticeData } from "./types";

type Value = NoticeData[keyof NoticeData];

const decimals = new Intl.NumberFormat("es-ES", { maximumFractionDigits: 1 });

/** A notice's number, which the hub sends as a decimal string. */
function amount(value: Value): number | undefined {
  const parsed = typeof value === "string" ? Number(value) : value;
  return typeof parsed === "number" && Number.isFinite(parsed) ? parsed : undefined;
}

/** A number as the person reads it: «40.000». */
function figure(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "?" : decimals.format(parsed);
}

/** A code as it was sent, without grouping: «-32603». */
function code(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "?" : String(parsed);
}

/** Milliseconds as seconds: «10 s», «2,5 s». */
function seconds(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "un tiempo desconocido" : `${decimals.format(parsed / 1000)} s`;
}

/** « (HTTP 503)», or nothing when the hub did not say. */
function http(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "" : ` (HTTP ${parsed})`;
}

const ASK_ADMIN = "Vuelve a intentarlo; si se repite, avisa a quien administra el hub.";
const NARROWER = "Pide algo más concreto o divídelo en pasos.";

/** A turn the hub refused for its files, in the words of the rule that fired. */
function attachmentFailure(data: NoticeData): string | undefined {
  switch (data.code) {
    case "attachments_too_large":
      return sessionCeilingText(amount(data.stored), amount(data.bytes), amount(data.max_bytes));
    case "attachments_too_many":
      return tooManyText(amount(data.count) ?? "?", amount(data.max) ?? "?");
    case "attachment_not_found":
      return ATTACHMENT_NOT_FOUND;
    case "attachment_refused":
      return "El agente no aceptó una de las imágenes de este mensaje; quítala o prueba con otra.";
    case "attachment_unreadable":
      return "El hub no pudo preparar uno de los adjuntos de este mensaje para el agente; quítalo o vuelve a adjuntarlo.";
    default:
      return undefined;
  }
}

/**
 * A turn the generic agent's provider ended without an answer
 * (hub-v1: `assistant_*`), with its numbers and what to do.
 */
function providerFailure(data: NoticeData, name: string): string | undefined {
  const provider = `El proveedor de ${name}`;
  switch (data.code) {
    case "assistant_unreachable":
      return `No se pudo conectar con el proveedor de ${name} en ${seconds(data.timeout_ms)}. Comprueba que el servidor del modelo está en marcha y que el hub llega a él.`;
    case "assistant_connect_timeout":
      return `${provider} no aceptó la conexión en ${seconds(data.timeout_ms)}. Vuelve a intentarlo; si se repite, revisa el servidor del modelo.`;
    case "assistant_timeout":
      return `${provider} dejó de contestar durante ${seconds(data.timeout_ms)}. Vuelve a preguntar; si se repite, revisa el servidor del modelo.`;
    case "assistant_connection_lost":
      return `Se cortó la conexión con el proveedor de ${name} antes de terminar la respuesta. Vuelve a preguntar.`;
    case "assistant_unauthorized":
      return `${provider} rechazó la clave${http(data.provider_status)}. Quien administra el hub debe revisar la clave del proveedor (api_key_file).`;
    case "assistant_http_error":
      return `${provider} respondió con un error${http(data.provider_status)}. ${ASK_ADMIN}`;
    case "assistant_rate_limited": {
      const limited = `${provider} está limitando las peticiones${http(data.provider_status)}`;
      return amount(data.retry_after_ms) === undefined
        ? `${limited} y no dijo cuánto esperar. Vuelve a preguntar dentro de un rato.`
        : `${limited} y pide esperar ${seconds(data.retry_after_ms)}. Vuelve a preguntar pasado ese tiempo.`;
    }
    case "assistant_context_exceeded": {
      const fits = `La conversación no cabe en el modelo de ${name}`;
      return amount(data.tokens) === undefined
        ? `${fits} (${figure(data.history_chars)} caracteres de historial). Abre una conversación nueva.`
        : `${fits}: ${figure(data.tokens)} tokens de un máximo de ${figure(data.max_tokens)}. Abre una conversación nueva.`;
    }
    case "assistant_tool_failed":
      // Only a tool that never answered carries its deadline; what it did is unknown.
      return amount(data.timeout_ms) === undefined
        ? `${name} no pudo usar una herramienta de Orbit y se detuvo. Vuelve a preguntar; si se repite, avisa a quien administra el hub.`
        : `Una herramienta de Orbit no respondió en ${seconds(data.timeout_ms)} y ${name} se detuvo. No se sabe si llegó a hacer lo que pedía: compruébalo antes de repetirlo.`;
    case "assistant_turn_too_long":
      return `${name} hizo ${figure(data.max_requests)} peticiones al modelo sin terminar y se detuvo. ${NARROWER}`;
    case "assistant_provider_error":
      return `${provider} falló sin decir por qué. Vuelve a intentarlo.`;
    case "assistant_failed":
      return `${name} falló antes de terminar la respuesta. Vuelve a preguntar.`;
    default:
      return undefined;
  }
}

/** Why a Claude Code or Codex agent left its turn unfinished (ACP's stop reasons). */
function unfinished(stopReason: Value, name: string): string {
  switch (stopReason) {
    case "max_tokens":
      return `${name} llegó a su máximo de tokens de respuesta antes de terminar. Pide una respuesta más corta o divide la pregunta.`;
    case "max_turn_requests":
      return `${name} llegó a su máximo de peticiones al modelo en un turno antes de terminar. ${NARROWER}`;
    case "refusal":
      return `${name} se negó a seguir con este turno.`;
    default:
      return `${name} paró el turno sin terminarlo.`;
  }
}

/**
 * A turn a Claude Code or Codex agent (hub-v1: `agent_*`) or Hermes
 * (`upstream_turn_failed`) ended without an answer.
 */
function agentFailure(data: NoticeData, name: string): string | undefined {
  switch (data.code) {
    case "agent_auth_required":
      return `${name} no tiene la sesión iniciada en el servidor del hub. Quien administra el hub debe iniciarla allí (Orbit no inicia sesiones).`;
    case "agent_rejected":
      return `${name} rechazó el turno (código ${code(data.acp_code)}). ${ASK_ADMIN}`;
    case "agent_stopped": {
      if (amount(data.exit_status) !== undefined)
        return `${name} se detuvo durante el turno (código de salida ${code(data.exit_status)}). Vuelve a preguntar.`;
      const ceiling = amount(data.max_bytes);
      if (ceiling !== undefined)
        return `${name} envió un mensaje de más de ${fileSize(ceiling)}, el máximo, y se detuvo. Vuelve a preguntar.`;
      return `${name} se detuvo durante el turno. Vuelve a preguntar.`;
    }
    case "agent_protocol":
      return `${name} respondió algo que el hub no entiende y se detuvo. Vuelve a preguntar; si se repite, avisa a quien administra el hub.`;
    case "agent_turn_unfinished":
      return unfinished(data.stop_reason, name);
    case "upstream_turn_failed":
      return `${name} terminó el turno sin una respuesta final. Vuelve a preguntar si hace falta.`;
    default:
      return undefined;
  }
}

/**
 * A failed turn in the words of its cause, naming the conversation's agent;
 * a cause without words of its own stays the generic failure with its code.
 */
export function assistantFailed(assistantName: string): Entries<"assistant_failed"> {
  return {
    assistant_failed: {
      class: "failed",
      text: (data) =>
        attachmentFailure(data) ??
        providerFailure(data, assistantName) ??
        agentFailure(data, assistantName),
    },
  };
}
