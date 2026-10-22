import { describe, expect, it } from "vitest";
import type { Refusal } from "../../bridge/refusal";
import { attachmentsCopy, fileSize } from "./attachments";
import { refusalEntries } from "./refusalEntries";

// Every attachment refusal names the file and the numbers of the rule that
// fired, so the person can fix the attachment instead of guessing.

function text(cause: Record<string, unknown>): string {
  const entry = refusalEntries.attachment;
  if (entry.class !== "fixable") throw new Error("an attachment refusal is the user's to fix");
  const refusal: Refusal = { code: "attachment", cause, retry: { kind: "after_fix" } };
  return entry.text(refusal);
}

describe("file sizes", () => {
  it("reads as B, KB or MB with a Spanish decimal comma", () => {
    expect(fileSize(512)).toBe("512 B");
    expect(fileSize(1_536)).toBe("1,5 KB");
    expect(fileSize(20 * 1024 * 1024)).toBe("20 MB");
    expect(fileSize(Math.round(2.25 * 1024 * 1024))).toBe("2,3 MB");
  });
});

describe("attachment refusals", () => {
  it("reads as Spanish when the hub names no file", () => {
    // The hub's `attachment_unreadable` for a turn's PDF pages carries only
    // its cause, no `name`.
    expect(text({ kind: "unreadable", reason: "render_failed" })).toBe(
      "El hub no pudo convertir en imágenes las páginas del fichero.",
    );
    expect(text({ kind: "unreadable", reason: "invalid_bytes" })).toBe(
      "El contenido del fichero está dañado o no corresponde a su tipo.",
    );
    expect(text({ kind: "unreadable", name: "plano.pdf", reason: "render_failed" })).toBe(
      "El hub no pudo convertir en imágenes las páginas de “plano.pdf”.",
    );
  });

  it("a hub without attachments asks to update it", () => {
    expect(text({ kind: "unsupported" })).toBe("Este hub no admite adjuntos; actualízalo.");
  });

  it("says why a HEIC photo could not be turned into JPEG", () => {
    expect(text({ kind: "unreadable", name: "IMG_0001.HEIC", reason: "photo_unreadable" })).toBe(
      "macOS no pudo leer la foto “IMG_0001.HEIC” para pasarla a JPEG.",
    );
    expect(text({ kind: "unreadable", name: "IMG_0001.HEIC", reason: "photo_jpeg_failed" })).toBe(
      "macOS no pudo pasar “IMG_0001.HEIC” a JPEG.",
    );
  });

  it("names the type no agent reads and what can be attached", () => {
    expect(text({ kind: "type_unsupported", name: "foto.heic", mime: "image/heic" })).toBe(
      "“foto.heic” es image/heic: ningún agente lo lee. Se pueden adjuntar imágenes PNG, JPEG, GIF o WebP, PDF y ficheros de texto.",
    );
  });

  it("explains each ceiling with its numbers", () => {
    expect(
      text({
        kind: "too_large",
        ceiling: "file",
        name: "copia.pdf",
        bytes: 25 * 1024 * 1024,
        max_bytes: 20 * 1024 * 1024,
      }),
    ).toBe("“copia.pdf” ocupa 25 MB y el máximo por fichero es 20 MB.");
    expect(
      text({
        kind: "too_large",
        ceiling: "message",
        files: 3,
        bytes: 40 * 1024 * 1024,
        max_bytes: 32 * 1024 * 1024,
      }),
    ).toBe("Los 3 adjuntos de este mensaje ocupan 40 MB y el máximo por mensaje es 32 MB.");
    expect(
      text({
        kind: "too_large",
        ceiling: "session",
        bytes: 10 * 1024 * 1024,
        session_bytes: 250 * 1024 * 1024,
        max_bytes: 256 * 1024 * 1024,
      }),
    ).toBe(
      "Esta conversación ya guarda 250 MB de adjuntos y este mensaje añade 10 MB; el máximo por conversación es 256 MB. Abre una conversación nueva para adjuntar más.",
    );
    expect(text({ kind: "too_many", files: 11, max_files: 10 })).toBe(
      "Este mensaje llevaría 11 adjuntos y el máximo por mensaje es 10.",
    );
  });

  it("says why a file could not be read", () => {
    expect(text({ kind: "unreadable", name: "ruta.pdf", reason: "pdf_locked" })).toBe(
      "“ruta.pdf” está protegido con contraseña.",
    );
    expect(text({ kind: "unreadable", name: "acta.txt", reason: "missing" })).toBe(
      "“acta.txt” ya no está donde estaba.",
    );
    expect(text({ kind: "unreadable", reason: "permission" })).toBe(
      "Orbit no tiene permiso para leer el fichero.",
    );
  });

  it("carries the agent's code when it refused an image", () => {
    expect(text({ kind: "refused", name: "x.png", upstream_code: 4018 })).toBe(
      "El agente no aceptó la imagen “x.png” (código 4018).",
    );
  });

  it("an attachment the hub no longer keeps says why and what to do", () => {
    expect(text({ kind: "not_found", attachment_id: "adjunto-1" })).toBe(
      "Ese adjunto ya no está en el hub (se borra si no se envía en una hora); vuelve a adjuntarlo.",
    );
  });
});

describe("the window's text", () => {
  const missing = attachmentsCopy.attachments.windowTextMissing;
  it("says when the capture goes alone, and why", () => {
    expect(missing("empty")).toBe("La ventana no tiene texto legible; va sólo la captura.");
    expect(missing("accessibility_permission_required")).toBe(
      "Sin el permiso de Accesibilidad, Orbit no lee su texto; va sólo la captura.",
    );
    expect(missing("timeout")).toBe("Su app no dio el texto a tiempo; va sólo la captura.");
    expect(missing("engine_failed")).toBe(
      "No se pudo leer su texto (causa engine_failed); va sólo la captura.",
    );
  });
});
