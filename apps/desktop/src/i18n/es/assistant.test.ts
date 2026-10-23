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

  it("says a failed transcription calmly and keeps the hub's numbers in its detail", () => {
    const failure = {
      code: "provider_timeout",
      hub_status: 504,
      provider_status: null,
      timeout_ms: 30000,
      bytes: null,
      max_bytes: null,
      attempts: 2,
    };
    expect(es.assistant.voiceFailure(failure)).toBe(
      "No se pudo transcribir. La nota sigue guardada en el Mac: reinténtalo o descártala.",
    );
    expect(es.assistant.voiceFailureDetail(failure)).toBe(
      "código provider_timeout, hub 504, límite 30 s, 2 intentos",
    );
  });

  it("says the provider did not accept the audio instead of offering what cannot work", () => {
    expect(
      es.assistant.voiceFailure({ code: "transcription_provider_refused", provider_status: 400 }),
    ).toBe(
      "El servicio de transcripción no aceptó este audio. Escúchala y, si hace falta, dilo otra vez o descártala.",
    );
  });
});
