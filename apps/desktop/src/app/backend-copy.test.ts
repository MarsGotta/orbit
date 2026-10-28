import { afterEach, describe, expect, it } from "vitest";
import { configureCopy, getCopy } from "../i18n";
import { receive } from "./interceptor";
import { request } from "../features/vault/api";
import type { Refusal } from "../bridge/refusal";

afterEach(() => configureCopy("es", ["es-ES"], "Deimos"));

describe.each(["es", "en"] as const)("backend copy in %s", (language) => {
  it("renders a daemon refusal from its code and numbers", () => {
    configureCopy(language, [], "Deimos");
    const result = receive([], {
      kind: "refusal",
      op: "invented-op",
      refusal: {
        code: "too_large",
        resource: "recording",
        bytes: 125,
        max_bytes: 100,
        retry: { kind: "after_fix" },
        message: "UNTRUSTED BACKEND TEXT",
      },
    });
    expect(result.messages[0]?.text).toContain(language === "es" ? "grabación" : "recording");
    expect(result.messages[0]?.text).toContain("125");
    expect(result.messages[0]?.text).toContain("100");
    expect(result.messages[0]?.text).toContain(language === "es" ? "máximo" : "maximum");
    expect(result.messages[0]?.text).not.toContain("UNTRUSTED");
  });

  it("renders a hub vault refusal and its bounds without reading message", async () => {
    configureCopy(language, [], "Deimos");
    await expect(
      request(
        async () => ({
          outcome: "refused",
          code: "vault_note_too_large",
          message: "UNTRUSTED BACKEND TEXT",
          numbers: { bytes: 125, max_bytes: 100 },
        }),
        { call: "disconnect" },
      ),
    ).rejects.toThrow(
      language === "es"
        ? "La nota ocupa 125 bytes; el máximo es 100."
        : "The note is 125 bytes; the maximum is 100.",
    );
  });

  it("names an unknown daemon code in local generic copy", () => {
    configureCopy(language, [], "Deimos");
    const result = receive([], {
      kind: "refusal",
      op: "invented-op",
      refusal: {
        code: "future_refusal",
        retry: { kind: "never" },
        message: "UNTRUSTED",
      } as unknown as Refusal,
    });
    expect(result.messages[0]?.text).toContain("future_refusal");
    expect(result.messages[0]?.text).toContain(language === "es" ? "reconoce" : "recognise");
    expect(result.messages[0]?.text).not.toContain("UNTRUSTED");
  });

  it("ignores unknown dictionary detail and names its code", () => {
    configureCopy(language, [], "Deimos");
    const text = getCopy().dictionary.refused("future_dictionary", "UNTRUSTED", { bytes: 25 });
    expect(text).toContain("future_dictionary");
    expect(text).toContain("25");
    expect(text).not.toContain("UNTRUSTED");
  });
});

it("rewords a retained vault refusal without repeating its request", async () => {
  configureCopy("es", [], "Deimos");
  let error: unknown;
  try {
    await request(
      async () => ({
        outcome: "refused",
        code: "vault_folder_missing",
        message: "UNTRUSTED",
        numbers: {},
      }),
      { call: "disconnect" },
    );
  } catch (caught) {
    error = caught;
  }
  expect(error).toBeInstanceOf(Error);
  expect((error as Error).message).toContain("La carpeta");
  configureCopy("en", [], "Deimos");
  expect((error as Error).message).toContain("The vault folder");
});

it("renders a working folder cause code without inserting its compatibility text", () => {
  for (const language of ["es", "en"] as const) {
    configureCopy(language, [], "Deimos");
    const words = getCopy().extras.agentRefusalText({
      code: "invalid_working_folder",
      status: 400,
      numbers: { cause_code: "working_folder_absolute", cause: "UNTRUSTED" },
    });
    expect(words).toContain(language === "es" ? "ruta absoluta" : "absolute hub path");
    expect(words).not.toContain("UNTRUSTED");
  }
});

it.each(["es", "en"] as const)(
  "renders a vault validation rule and its field in %s",
  async (language) => {
    configureCopy(language, [], "Deimos");
    await expect(
      request(
        async () => ({
          outcome: "refused",
          code: "vault_invalid_request",
          message: "UNTRUSTED",
          message_fields: { field: "page", rule: "greater_than_equal" },
          numbers: { max_page_size: 100 },
        }),
        { call: "disconnect" },
      ),
    ).rejects.toThrow(
      language === "es"
        ? "El campo «page» es menor que el mínimo. Las páginas admiten de 1 a 100 entradas."
        : "Field “page” is below the minimum. Pages allow 1 to 100 entries.",
    );
  },
);

it.each(["es", "en"] as const)(
  "does not display an older hub's read-only explanation in %s",
  (language) => {
    configureCopy(language, [], "Deimos");
    const text = getCopy().agents.notEditable("UNTRUSTED");
    expect(text).toBe(
      language === "es"
        ? "Este tipo de agente no deja cambiarlo desde Orbit."
        : "This agent type can’t be changed from Orbit.",
    );
  },
);

it.each(["es", "en"] as const)(
  "unknown hub copy keeps numbers without fallback prose in %s",
  (language) => {
    configureCopy(language, [], "Deimos");
    const text = getCopy().extras.agentRefusalText({
      code: "future_hub_code",
      status: 409,
      numbers: { cause: "UNTRUSTED", count: 7 },
    });
    expect(text).toContain("future_hub_code");
    expect(text).toContain("7");
    expect(text).not.toContain("UNTRUSTED");
  },
);
