import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { VoiceTranscriptionFailure } from "../../bridge/generated/VoiceTranscriptionFailure";
import { es } from "../../i18n/es";
import { withPendingVoice } from "./conversation";

function failure(code: string): VoiceTranscriptionFailure {
  return {
    code,
    provider_status: null,
    hub_status: null,
    timeout_ms: null,
    bytes: null,
    max_bytes: null,
  };
}

function message(id: string, voice: ChatMessage["voice"] = null): ChatMessage {
  return {
    id,
    role: "user",
    text: "",
    at_ms: 1,
    streaming: false,
    voice,
    receipts: [],
    attachments: [],
  };
}

const waiting = message("nota-sintetica", {
  duration_ms: 4_000,
  failure: failure("voice_hub_unreachable"),
  audio_available: true,
});

describe("voice notes kept for the hub", () => {
  it("are shown after the conversation, once each", () => {
    const stored = [message("pregunta-sintetica")];
    expect(withPendingVoice(stored, [waiting])).toEqual([...stored, waiting]);
    expect(withPendingVoice([...stored, waiting], [waiting])).toEqual([...stored, waiting]);
    expect(withPendingVoice(stored, [])).toBe(stored);
  });

  it("say they wait on the Mac for the hub, not that they were lost", () => {
    expect(es.assistant.voiceFailure(failure("voice_hub_unreachable"))).toBe(
      "Guardada en el Mac: se enviará sola cuando vuelva la conexión con el hub.",
    );
    expect(es.assistant.voiceFailure(failure("voice_waiting_turn"))).toContain(
      "Guardada en el Mac",
    );
  });
});
