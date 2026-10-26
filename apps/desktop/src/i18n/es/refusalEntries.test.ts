import { describe, expect, it } from "vitest";
import { refusalEntries } from "./refusalEntries";

describe("permission refusals", () => {
  it("a permission not yet observed is not presented as missing and does not ask to be granted", () => {
    const entry = refusalEntries.permission_unknown;
    expect(entry.class).toBe("working");
    if (entry.class === "failed")
      throw new Error("the pending observation was presented as a failure");
    const text = entry.text({
      code: "permission_unknown",
      permission: "accessibility",
      retry: { kind: "after_fix" },
    });
    expect(text).toContain("todavía no sabe");
    expect(text).not.toContain("Falta");
    expect(text).not.toContain("Concédelo");
    expect(entry.action).toBeUndefined();
  });
});

it("the calendar refusal keeps its limits readable in Spanish", () => {
  const entry = refusalEntries.calendar_read_limit;
  expect(
    entry.text?.({
      code: "calendar_read_limit",
      retry: { kind: "after_fix" },
      cause: {
        kind: "fields",
        max_title_bytes: 4096,
        max_label_bytes: 512,
        max_timezone_bytes: 128,
      },
    }),
  ).toBe(
    "La lectura del calendario supera sus límites: título de hasta 4096 bytes, etiqueta de hasta 512 bytes y zona horaria de hasta 128 bytes. Pide un rango más corto.",
  );
});

it("the generation that interrupted a read reaches the person", () => {
  const entry = refusalEntries.read_interrupted;
  expect(
    entry.text?.({
      code: "read_interrupted",
      retry: { kind: "check_outcome" },
      cause: { kind: "generation_changed", started_generation: 7, current_generation: 8 },
    }),
  ).toBe(
    "La lectura empezó en la generación 7 y el Mac está en la 8. Vuelve a pedirla si sigue haciendo falta.",
  );
});

it.each(["audio", "voice_transcription"])(
  "%s is a calm wait without a failure code",
  (operation) => {
    const entry = refusalEntries.busy;
    expect(entry.class).toBe("working");
    expect(entry.text?.({ code: "busy", operation, retry: { kind: "after_fix" } })).toBe(
      operation === "audio"
        ? "Espera a que termine la nota de voz."
        : "Espera a que termine la transcripción de la nota de voz.",
    );
  },
);
