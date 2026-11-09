import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { VoiceNote as VoiceNoteSnapshot } from "../../bridge/generated/VoiceNote";
import { VoiceNote } from "./VoiceNote";

afterEach(cleanup);

function failed(code: string, provider_status: number | null = null): VoiceNoteSnapshot {
  return {
    duration_ms: 1_500,
    failure: {
      code,
      provider_status,
      hub_status: 502,
      timeout_ms: null,
      bytes: null,
      max_bytes: null,
    },
    audio_available: true,
  };
}

function show(voice: VoiceNoteSnapshot) {
  const send = vi.fn<(command: AppCommand) => void>();
  render(<VoiceNote messageId="nota-sintetica" send={send} voice={voice} />);
  return send;
}

describe("a voice note that could not be transcribed", () => {
  it("is discarded only once the question in place is answered", () => {
    const send = show(failed("transcription_hub_timeout"));
    fireEvent.click(screen.getByRole("button", { name: "Descartar" }));
    expect(send).not.toHaveBeenCalled();
    expect(screen.getByText("¿Descartar esta nota de voz?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Descartar" }));
    const question = screen.getByRole("dialog", { name: "¿Descartar esta nota de voz?" });
    fireEvent.click(within(question).getByRole("button", { name: "Descartar" }));
    expect(send).toHaveBeenCalledWith({ command: "voice_discard", message_id: "nota-sintetica" });
  });

  it("offers no retry when the provider refused its audio, only «Descartar»", () => {
    for (const voice of [
      failed("transcription_provider_refused", 400),
      // As a 0.11.0 hub said it.
      failed("transcription_http_error", 400),
    ]) {
      show(voice);
      expect(screen.queryByRole("button", { name: "Reintentar transcripción" })).toBeNull();
      expect(screen.getByRole("button", { name: "Descartar" })).toBeTruthy();
      expect(screen.getByText(/no aceptó este audio/)).toBeTruthy();
      cleanup();
    }
  });

  it("says it calmly, without codes, and keeps them in the detail", () => {
    show(failed("transcription_hub_timeout"));
    const line = screen.getByText(/No se pudo transcribir/);
    expect(line.textContent).not.toMatch(/código|hub 502/);
    expect(line.getAttribute("title")).toBe("código transcription_hub_timeout, hub 502");
    expect(screen.getByRole("button", { name: "Reintentar transcripción" })).toBeTruthy();
  });

  it("can be discarded while it waits to be sent by itself, not while transcribed", () => {
    show(failed("voice_hub_unreachable"));
    expect(screen.getByRole("button", { name: "Descartar" })).toBeTruthy();
    cleanup();
    show({ duration_ms: 1_500, audio_available: true });
    expect(screen.queryByRole("button", { name: "Descartar" })).toBeNull();
  });
});
