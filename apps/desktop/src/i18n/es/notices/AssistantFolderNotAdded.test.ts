import { describe, expect, it } from "vitest";
import { notices } from ".";

describe("a folder chosen in the panel that Orbit did not keep", () => {
  const entry = notices.assistant_folder_not_added;

  it("is something to fix, said with its cause and numbers", () => {
    expect(entry.class).toBe("fixable");
    expect(
      entry.text({
        code: "folder_unavailable",
        cause: "bookmark_too_large",
        bytes: 16_398,
        max_bytes: 16_384,
      }),
    ).toBe(
      "No se añadió la carpeta: el marcador que macOS hizo de ella ocupa 16.398 bytes y Orbit guarda hasta 16.384.",
    );
    expect(
      entry.text({
        code: "folder_unavailable",
        cause: "path_too_long",
        bytes: 1100,
        max_bytes: 1024,
      }),
    ).toBe("No se añadió la carpeta: su ruta ocupa 1100 bytes y Orbit admite 1024.");
    expect(entry.text({ code: "too_many", count: 9, max: 8 })).toBe(
      "No se añadió la carpeta: ya hay 8 carpetas, el máximo (sería la 9.ª). Quita una para añadir otra.",
    );
  });

  it("says the panel's own failure with the helper's code and the system's number", () => {
    expect(entry.text({ code: "native_operation_failed", status: 256 })).toBe(
      "No se pudo añadir la carpeta: el panel de macOS no la entregó (native_operation_failed, error 256). Vuelve a intentarlo.",
    );
    expect(entry.text({ code: "storage_failed" })).toBe(
      "No se añadió la carpeta: Orbit no pudo guardarla en este Mac. Vuelve a añadirla.",
    );
  });
});
