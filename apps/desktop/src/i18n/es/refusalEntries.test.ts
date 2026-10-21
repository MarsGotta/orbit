import { describe, expect, it } from "vitest";
import { refusalEntries } from "./refusalEntries";

describe("negativas de permisos", () => {
  it("un permiso sin observar no se presenta como una falta ni pide concederlo", () => {
    const entry = refusalEntries.permission_unknown;
    expect(entry.class).toBe("working");
    if (entry.class === "failed")
      throw new Error("la observación pendiente se presentó como fallo");
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

it("la negativa de calendario conserva límites legibles en español", () => {
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

it("la generación que interrumpió una lectura llega a la persona", () => {
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
