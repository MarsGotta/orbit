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
