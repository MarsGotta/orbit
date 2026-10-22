import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { VoiceNote } from "../../bridge/generated/VoiceNote";
import { genericFailure, receive } from "../../app/interceptor";
import { MessageList } from "./MessageList";

// «Reintentar transcripción» on a voice note whose transcription failed asks
// the daemon to transcribe again the audio this Mac keeps; without that audio
// there is nothing to retry, and the note says the audio is not here.

function note(id: string, voice: VoiceNote): ChatMessage {
  return { id, role: "user", text: "", at_ms: 1, streaming: false, voice, receipts: [] };
}

const failed = {
  code: "transcription_upstream_failed",
  provider_status: 503,
  hub_status: 502,
  timeout_ms: null,
  bytes: null,
  max_bytes: null,
};

afterEach(cleanup);

describe("a voice note whose transcription failed", () => {
  it("offers «Reintentar transcripción», which asks for it with the note's id", () => {
    const send = vi.fn<(command: AppCommand) => void>();
    render(
      <MessageList
        messages={[note("voice-1", { duration_ms: 3_000, failure: failed, audio_available: true })]}
        send={send}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Reintentar transcripción" }));
    expect(send).toHaveBeenCalledWith({ command: "voice_retry", message_id: "voice-1" });
  });

  it("does not offer it when this Mac no longer has the audio", () => {
    render(
      <MessageList
        messages={[
          note("voice-2", { duration_ms: 3_000, failure: failed, audio_available: false }),
        ]}
        send={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "Reintentar transcripción" })).toBeNull();
    expect(screen.getByRole("button", { name: "Audio no disponible" })).toBeTruthy();
  });

  it("does not offer it for a note that was transcribed or is being transcribed", () => {
    render(
      <MessageList
        messages={[
          note("voice-3", { duration_ms: 3_000, transcript: "Hola", audio_available: true }),
          note("voice-4", { duration_ms: 3_000, audio_available: true }),
        ]}
        send={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "Reintentar transcripción" })).toBeNull();
  });
});

describe("a retry whose audio is gone", () => {
  it("says the audio is no longer on this Mac", () => {
    const { messages } = receive(
      [],
      {
        kind: "refusal",
        op: "op-retry-1",
        refusal: { code: "not_found", resource: "audio", retry: { kind: "after_fix" } },
      },
      { place: "assistant" },
    );
    expect(messages.at(-1)?.text).toBe(
      "El audio de esa nota de voz ya no está en este Mac: no se puede volver a transcribir. Grábala otra vez.",
    );
  });

  it("leaves any other missing thing as the generic failure with its code", () => {
    const { messages } = receive(
      [],
      {
        kind: "refusal",
        op: "op-retry-2",
        refusal: { code: "not_found", resource: "session", retry: { kind: "after_fix" } },
      },
      { place: "assistant" },
    );
    expect(messages.at(-1)?.text).toBe(genericFailure("op-retry-2"));
  });
});
