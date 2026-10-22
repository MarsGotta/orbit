import { describe, expect, it } from "vitest";
import { notices } from ".";

describe("a cut recording", () => {
  it("that went on by itself says its cause and the audio missing", () => {
    const entry = notices.recording_resumed_after_cut;
    expect(entry.class).toBe("done");
    expect(entry.text({ cause: "daemon_heartbeat_expired", elapsed_ms: 11_300 })).toBe(
      "La grabación se cortó (el servicio de Orbit dejó de responder) y ha seguido sola; faltan 11 s de audio.",
    );
  });

  it("that could not go on says it was closed and what to do", () => {
    const entry = notices.recording_closed_after_cut;
    expect(entry.class).toBe("fixable");
    expect(entry.text({ cause: "session_inactive", fragments: 12 })).toBe(
      "La grabación se cortó (el Mac se durmió o cambió de usuario) y no pudo seguir. Se ha cerrado con lo grabado y su nota se escribirá; si la reunión sigue, empieza otra grabación.",
    );
    expect(entry.text({ cause: "capture_finalize_timed_out" })).toContain(
      "sin que el Mac confirmara su final",
    );
  });
});

describe("an upload the hub refuses", () => {
  it("until this Mac is paired again says so with its status and code", () => {
    const entry = notices.upload_refused_retrying;
    expect(entry.class).toBe("fixable");
    expect(entry.text({ status: 401, code: "unauthorized", step: "chunk_upload" })).toBe(
      "El hub no acepta este Mac (HTTP 401, unauthorized). Vuelve a emparejarlo en Ajustes: el audio se queda en el Mac y se sube después.",
    );
    expect(entry.text({ status: 404, step: "chunk_upload" })).toContain("HTTP 404");
  });

  it("for good says what stays on the Mac, never the generic failure", () => {
    const entry = notices.upload_refused;
    expect(entry.class).toBe("failed");
    expect(entry.text?.({ status: 409, code: "recording_finalized", step: "chunk_upload" })).toBe(
      "El hub rechazó un trozo de audio (HTTP 409, recording_finalized). Se queda en el Mac y no se reintenta; la reunión se cierra sin él.",
    );
  });

  it("for lack of space says the hub's numbers", () => {
    const entry = notices.hub_storage_full;
    expect(entry.class).toBe("fixable");
    const GIB = 1024 ** 3;
    expect(
      entry.text({
        code: "quota_exceeded",
        status: 507,
        bytes: 9.5 * GIB,
        max_bytes: 10 * GIB,
        remaining: 0.5 * GIB,
      }),
    ).toBe(
      "El hub no tiene espacio para más audio (usa 9,5 GB de 10 GB, quedan 512 MB). El audio se queda en el Mac y se sube cuando haya sitio: libera espacio en el hub o amplía su límite.",
    );
    // A disk the hub could not write to says what it has free.
    expect(entry.text({ code: "storage_full", status: 507, bytes: 80 * GIB, remaining: 0 })).toBe(
      "El hub no tiene espacio para más audio (usa 80 GB, quedan 0 B). El audio se queda en el Mac y se sube cuando haya sitio: libera espacio en el hub o amplía su límite.",
    );
  });
});
