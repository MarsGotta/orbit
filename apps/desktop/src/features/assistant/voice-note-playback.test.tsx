import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import { es } from "../../i18n/es";
import { VoiceNote } from "./VoiceNote";

// «Reproducir nota de voz» asks the app for the note's audio. A request that
// fails, or that brings no audio, leaves the button able to ask again.

const voice = { duration_ms: 3_000, transcript: "Hola", audio_available: true };

describe("playing a voice note", () => {
  it("can be asked again after a request that failed", async () => {
    const send = vi.fn<(command: AppCommand) => Promise<boolean>>(async () => false);
    render(<VoiceNote messageId="voice-1" send={send} voice={voice} />);
    const play = screen.getByRole("button", { name: es.assistant.playVoiceNote });
    await userEvent.click(play);
    await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
    await userEvent.click(play);
    await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
  });

  it("is asked once while its request travels", async () => {
    const send = vi.fn<(command: AppCommand) => Promise<boolean>>(
      () => new Promise<boolean>(() => undefined),
    );
    render(<VoiceNote messageId="voice-1" send={send} voice={voice} />);
    const play = screen.getByRole("button", { name: es.assistant.playVoiceNote });
    await userEvent.click(play);
    await userEvent.click(play);
    expect(send).toHaveBeenCalledTimes(1);
  });
});
