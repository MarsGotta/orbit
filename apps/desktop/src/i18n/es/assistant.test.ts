import { describe, expect, it } from "vitest";
import { es } from "./index";

describe("voice note failures", () => {
  it("names a cause Orbit stopped instead of asking the user to record again", () => {
    const text = es.assistant.voiceFailure({ code: "voice_transcription_joined_next_turn" });
    expect(text).toBe("Va junto con lo que dijiste a continuación.");
    expect(
      es.assistant.voiceFailure({ code: "voice_transcription_call_stopped_automatically" }),
    ).not.toContain("Vuelve a grabarla");
  });

  it("carries the hub's numbers when the transcription failed", () => {
    const text = es.assistant.voiceFailure({
      code: "provider_timeout",
      hub_status: 504,
      provider_status: null,
      timeout_ms: 30000,
      bytes: null,
      max_bytes: null,
      attempts: 2,
    });
    expect(text).toBe(
      "No se pudo transcribir la nota de voz (código provider_timeout, hub 504, límite 30 s, 2 intentos). Vuelve a grabarla.",
    );
  });
});
