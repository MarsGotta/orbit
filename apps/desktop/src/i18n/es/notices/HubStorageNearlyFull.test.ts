import { describe, expect, it } from "vitest";
import { notices } from ".";

const GIB = 1024 ** 3;

describe("a hub that is nearly full", () => {
  const entry = notices.hub_storage_nearly_full;

  it("says how much it uses of what, what is left and what to do on the server's disk", () => {
    expect(entry.class).toBe("fixable");
    expect(
      entry.text({
        bytes: 82 * GIB,
        max_bytes: 100 * GIB,
        remaining: 18 * GIB,
        cause: "disk",
      }),
    ).toBe(
      "El hub está casi lleno: ocupa 82 GB de 100 GB y quedan 18 GB. Libera espacio en el disco del servidor. Si se llena, las reuniones nuevas se quedan en este Mac sin transcribir hasta que haya sitio. El hub no borra nada por su cuenta.",
    );
  });

  it("names the hub's quota when the quota sets the limit", () => {
    expect(
      entry.text({
        bytes: 8.5 * GIB,
        max_bytes: 10 * GIB,
        remaining: 1.5 * GIB,
        cause: "quota",
      }),
    ).toContain(
      "ocupa 8,5 GB de 10 GB y quedan 1,5 GB. Libera espacio en el servidor o sube storage_quota_bytes en la configuración del hub.",
    );
  });

  it("says full when nothing is left", () => {
    expect(
      entry.text({ bytes: 10 * GIB, max_bytes: 10 * GIB, remaining: 0, cause: "quota" }),
    ).toMatch(/^El hub está lleno: ocupa 10 GB de 10 GB\. /);
  });
});
