import { describe, expect, it } from "vitest";
import { genericFailure, receive } from "../../../app/interceptor";
import type { Notice } from "../../../bridge/generated/Notice";

// A turn the hub refused for its files says which rule and its numbers;
// every other failure of a turn keeps the generic message with its code.

const OP = "5c1d2e3f-0000-4000-8000-000000000003";

function shown(data: Notice["data"]): string | undefined {
  const notice: Notice = { code: "assistant_failed", op: OP, data };
  return receive([], { kind: "notice", notice }).messages[0]?.text;
}

describe("a turn the hub refused for its attachments", () => {
  it("names the conversation's ceiling with its sizes", () => {
    expect(
      shown({
        code: "attachments_too_large",
        bytes: "120",
        stored: String(Math.round(255.9 * 1024 * 1024)),
        max_bytes: String(256 * 1024 * 1024),
      }),
    ).toBe(
      "Esta conversación ya guarda 255,9 MB de adjuntos y este mensaje añade 120 B; el máximo por conversación es 256 MB. Abre una conversación nueva para adjuntar más.",
    );
  });

  it("names the count and the per-message ceiling", () => {
    expect(shown({ code: "attachments_too_many", count: "11", max: "10" })).toBe(
      "Este mensaje llevaría 11 adjuntos y el máximo por mensaje es 10.",
    );
  });

  it("says what happened to a file the hub no longer has, refused or could not prepare", () => {
    expect(shown({ code: "attachment_not_found" })).toBe(
      "Ese adjunto ya no está en el hub (se borra si no se envía en una hora); vuelve a adjuntarlo.",
    );
    expect(shown({ code: "attachment_refused" })).toBe(
      "El agente no aceptó una de las imágenes de este mensaje; quítala o prueba con otra.",
    );
    expect(shown({ code: "attachment_unreadable" })).toBe(
      "El hub no pudo preparar uno de los adjuntos de este mensaje para el agente; quítalo o vuelve a adjuntarlo.",
    );
  });

  it("keeps the generic failure for every other cause", () => {
    expect(shown({ code: "provider_timeout" })).toBe(genericFailure(OP));
    expect(shown(undefined)).toBe(genericFailure(OP));
  });
});

// A turn that ended without an answer says why, naming the conversation's
// agent, with the numbers the hub sent, and what the person can do.

function said(data: Notice["data"], agent = "Lyra"): string | undefined {
  const notice: Notice = { code: "assistant_failed", op: OP, data };
  return receive([], { kind: "notice", notice }, {}, agent).messages[0]?.text;
}

describe("a turn the agent's provider ended without an answer", () => {
  it("names the connection's deadline when the provider is not there", () => {
    expect(said({ code: "assistant_unreachable", timeout_ms: "10000" })).toBe(
      "No se pudo conectar con el proveedor de Lyra en 10 s. Comprueba que el servidor del modelo está en marcha y que el hub llega a él.",
    );
    expect(said({ code: "assistant_connect_timeout", timeout_ms: "10000" })).toBe(
      "El proveedor de Lyra no aceptó la conexión en 10 s. Vuelve a intentarlo; si se repite, revisa el servidor del modelo.",
    );
  });

  it("names how long the provider stayed silent, or that the connection was cut", () => {
    expect(said({ code: "assistant_timeout", timeout_ms: "120000" })).toBe(
      "El proveedor de Lyra dejó de contestar durante 120 s. Vuelve a preguntar; si se repite, revisa el servidor del modelo.",
    );
    expect(said({ code: "assistant_connection_lost" })).toBe(
      "Se cortó la conexión con el proveedor de Lyra antes de terminar la respuesta. Vuelve a preguntar.",
    );
  });

  it("names the provider's HTTP status", () => {
    expect(said({ code: "assistant_unauthorized", provider_status: "401" })).toBe(
      "El proveedor de Lyra rechazó la clave (HTTP 401). Quien administra el hub debe revisar la clave del proveedor (api_key_file).",
    );
    expect(said({ code: "assistant_http_error", provider_status: "503" })).toBe(
      "El proveedor de Lyra respondió con un error (HTTP 503). Vuelve a intentarlo; si se repite, avisa a quien administra el hub.",
    );
  });

  it("says how long to wait when the provider limits requests, and when it did not say", () => {
    expect(
      said({ code: "assistant_rate_limited", provider_status: "429", retry_after_ms: "2500" }),
    ).toBe(
      "El proveedor de Lyra está limitando las peticiones (HTTP 429) y pide esperar 2,5 s. Vuelve a preguntar pasado ese tiempo.",
    );
    expect(said({ code: "assistant_rate_limited", provider_status: "429" })).toBe(
      "El proveedor de Lyra está limitando las peticiones (HTTP 429) y no dijo cuánto esperar. Vuelve a preguntar dentro de un rato.",
    );
  });

  it("names the model's context in tokens when the provider gave them, else the history sent", () => {
    expect(
      said({
        code: "assistant_context_exceeded",
        history_chars: "60000",
        tokens: "40000",
        max_tokens: "32768",
      }),
    ).toBe(
      "La conversación no cabe en el modelo de Lyra: 40.000 tokens de un máximo de 32.768. Abre una conversación nueva.",
    );
    expect(said({ code: "assistant_context_exceeded", history_chars: "60000" })).toBe(
      "La conversación no cabe en el modelo de Lyra (60.000 caracteres de historial). Abre una conversación nueva.",
    );
  });

  it("says a tool that never answered may have acted, and names the other tool failures", () => {
    expect(said({ code: "assistant_tool_failed", timeout_ms: "60000" })).toBe(
      "Una herramienta de Orbit no respondió en 60 s y Lyra se detuvo. No se sabe si llegó a hacer lo que pedía: compruébalo antes de repetirlo.",
    );
    expect(said({ code: "assistant_tool_failed" })).toBe(
      "Lyra no pudo usar una herramienta de Orbit y se detuvo. Vuelve a preguntar; si se repite, avisa a quien administra el hub.",
    );
  });

  it("names the requests a turn took, and the failures with no numbers", () => {
    expect(said({ code: "assistant_turn_too_long", max_requests: "50" })).toBe(
      "Lyra hizo 50 peticiones al modelo sin terminar y se detuvo. Pide algo más concreto o divídelo en pasos.",
    );
    expect(said({ code: "assistant_provider_error" })).toBe(
      "El proveedor de Lyra falló sin decir por qué. Vuelve a intentarlo.",
    );
    expect(said({ code: "assistant_failed" })).toBe(
      "Lyra falló antes de terminar la respuesta. Vuelve a preguntar.",
    );
  });
});

describe("a turn a Claude Code or Codex agent ended without an answer", () => {
  it("says the agent is signed out on the hub's server", () => {
    expect(said({ code: "agent_auth_required" }, "Vega")).toBe(
      "Vega no tiene la sesión iniciada en el servidor del hub. Quien administra el hub debe iniciarla allí (Orbit no inicia sesiones).",
    );
  });

  it("names the agent's error code, and that it answered what the hub cannot read", () => {
    expect(said({ code: "agent_rejected", acp_code: "-32603" }, "Vega")).toBe(
      "Vega rechazó el turno (código -32603). Vuelve a intentarlo; si se repite, avisa a quien administra el hub.",
    );
    expect(said({ code: "agent_protocol" }, "Vega")).toBe(
      "Vega respondió algo que el hub no entiende y se detuvo. Vuelve a preguntar; si se repite, avisa a quien administra el hub.",
    );
  });

  it("names why the agent stopped: its exit status, a message over the ceiling, or neither", () => {
    expect(said({ code: "agent_stopped", exit_status: "3" }, "Vega")).toBe(
      "Vega se detuvo durante el turno (código de salida 3). Vuelve a preguntar.",
    );
    expect(said({ code: "agent_stopped", max_bytes: String(8 * 1024 * 1024) }, "Vega")).toBe(
      "Vega envió un mensaje de más de 8 MB, el máximo, y se detuvo. Vuelve a preguntar.",
    );
    expect(said({ code: "agent_stopped" }, "Vega")).toBe(
      "Vega se detuvo durante el turno. Vuelve a preguntar.",
    );
  });

  it("names the stop reason of a turn the agent left unfinished", () => {
    expect(said({ code: "agent_turn_unfinished", stop_reason: "max_tokens" }, "Vega")).toBe(
      "Vega llegó a su máximo de tokens de respuesta antes de terminar. Pide una respuesta más corta o divide la pregunta.",
    );
    expect(said({ code: "agent_turn_unfinished", stop_reason: "max_turn_requests" }, "Vega")).toBe(
      "Vega llegó a su máximo de peticiones al modelo en un turno antes de terminar. Pide algo más concreto o divídelo en pasos.",
    );
    expect(said({ code: "agent_turn_unfinished", stop_reason: "refusal" }, "Vega")).toBe(
      "Vega se negó a seguir con este turno.",
    );
    expect(said({ code: "agent_turn_unfinished" }, "Vega")).toBe(
      "Vega paró el turno sin terminarlo.",
    );
  });
});

describe("a turn Hermes ended without a final answer", () => {
  it("names the conversation's agent", () => {
    expect(said({ code: "upstream_turn_failed" }, "Atlas")).toBe(
      "Atlas terminó el turno sin una respuesta final. Vuelve a preguntar si hace falta.",
    );
  });
});
