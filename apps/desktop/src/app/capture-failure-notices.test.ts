// What could not be recorded or saved is said with what happened, its cause
// when the notice carries one, and what to do: never «No se pudo …», the
// generic failure with only a code to share.
import { describe, expect, it } from "vitest";
import type { Notice } from "../bridge/generated/Notice";
import type { NoticeCode } from "../bridge/generated/NoticeCode";
import { failures } from "../i18n/es/failures";
import { receive } from "./interceptor";

const OP = "5e1d2c3b-0000-4000-8000-000000000042";

function shown(code: NoticeCode, data?: Notice["data"]): string {
  const notice: Notice = { code, op: OP, data };
  return receive([], { kind: "notice", notice }).messages.find((message) => message.op === OP)
    ?.text as string;
}

const generic = failures.failure.helpBefore;

describe("a cut or unsaved recording says what happened", () => {
  it.each([
    ["capture_cut", { cause: "helper_lost", recording: true, fragments: 3 }],
    ["capture_cut", { cause: "algo_que_esta_version_no_conoce" }],
    ["recording_seal_failed", {}],
    ["recording_seal_failed", { cause: "disk_full", fragments: 2 }],
    ["recording_close_unconfirmed", { fragments: true }],
    ["recording_close_unconfirmed", { fragments: false }],
    ["audio_start_refused", { code: "microphone_unavailable" }],
    ["audio_start_refused", {}],
    ["recording_interrupted_by_restart", { fragments: 12 }],
    ["store_recovery_lost_files", { recovered: 1, invalid: 1, missing: 2, incomplete: 0 }],
    ["closed_with_pending_work", { audio_unsaved: true, reserve_exhausted: false }],
    ["closed_with_pending_work", {}],
  ] as const)("%s %j has words of its own", (code, data) => {
    const text = shown(code, data as Notice["data"]);
    expect(text).toBeTruthy();
    expect(text).not.toContain(generic);
    expect(text).not.toMatch(/^No se pudo /);
  });

  it("a cut names its cause and what was kept", () => {
    const text = shown("capture_cut", { cause: "helper_lost", recording: true, fragments: 3 });
    expect(text).toContain("La grabación se cortó");
    expect(text).toContain("dejó de responder");
    expect(text).toContain("3 fragmentos");
  });

  it("a cut whose cause this version does not know still names it, by its code", () => {
    expect(shown("capture_cut", { cause: "algo_que_esta_version_no_conoce" })).toContain(
      "algo_que_esta_version_no_conoce",
    );
  });

  it("a seal that failed says why, what is still waiting and what to do", () => {
    const text = shown("recording_seal_failed", { cause: "disk_full", fragments: 2 });
    expect(text).toContain("el disco de este Mac está lleno");
    expect(text).toContain("2 fragmentos");
    expect(text).toContain("Libera espacio");
    expect(shown("recording_seal_failed", { cause: "otra_causa_nueva" })).toContain(
      "otra_causa_nueva",
    );
  });

  it("a close the Mac never confirmed says what was kept and that the note is written", () => {
    const kept = shown("recording_close_unconfirmed", { fragments: true });
    expect(kept).toContain("no confirmó a tiempo el final de la grabación");
    expect(kept).toContain("el hub escribe su nota");
    expect(shown("recording_close_unconfirmed", { fragments: false })).toContain(
      "No llegó a guardarse audio",
    );
  });

  it("a microphone that could not open names the helper's code and what to do", () => {
    const text = shown("audio_start_refused", { code: "microphone_unavailable" });
    expect(text).toContain("no pudo abrir el micrófono");
    expect(text).toContain("microphone_unavailable");
    expect(text).toContain("Ajustes del Sistema");
  });

  it("a recovery that lost files counts them", () => {
    const text = shown("store_recovery_lost_files", {
      recovered: 1,
      invalid: 1,
      missing: 2,
      incomplete: 0,
    });
    expect(text).toContain("3 archivos de audio");
    expect(text).toContain("recuperó 1");
  });

  it("closing with audio not yet written says that part of it was lost", () => {
    expect(shown("closed_with_pending_work", { audio_unsaved: true })).toContain("parte del audio");
  });
});
