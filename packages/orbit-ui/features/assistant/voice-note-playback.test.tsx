import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

// It sounds only when pressed, and the same button stops it: a note shown
// again with its audio already here (a conversation opened again, a page
// drawn again) stays silent.
describe("a voice note's playback", () => {
  const audio = { message_id: "voice-1", format: "wav", bytes_base64: "UklGRg==" };

  function media(): HTMLAudioElement {
    return screen.getByTestId("voice-audio-voice-1") as HTMLAudioElement;
  }

  it("never starts by itself when the note is shown with its audio", async () => {
    // jsdom has no media playback.
    const played = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    const { rerender } = render(
      <VoiceNote audio={audio} messageId="voice-1" send={() => true} voice={voice} />,
    );
    await waitFor(() => expect(media().getAttribute("src")).toBe("data:audio/wav;base64,UklGRg=="));
    rerender(
      <VoiceNote audio={{ ...audio }} messageId="voice-1" send={() => true} voice={voice} />,
    );
    expect(played).not.toHaveBeenCalled();
    played.mockRestore();
  });

  it("plays the audio that arrives after it was pressed", async () => {
    const played = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    const send = vi.fn<(command: AppCommand) => Promise<boolean>>(async () => true);
    const { rerender } = render(<VoiceNote messageId="voice-1" send={send} voice={voice} />);
    await userEvent.click(screen.getByRole("button", { name: es.assistant.playVoiceNote }));
    expect(played).not.toHaveBeenCalled();
    rerender(<VoiceNote audio={audio} messageId="voice-1" send={send} voice={voice} />);
    await waitFor(() => expect(played).toHaveBeenCalledTimes(1));
    played.mockRestore();
  });

  it("stops with the same button, back at its start", async () => {
    const played = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    const paused = vi
      .spyOn(HTMLMediaElement.prototype, "pause")
      .mockImplementation(() => undefined);
    render(<VoiceNote audio={audio} messageId="voice-1" send={() => true} voice={voice} />);
    await waitFor(() => expect(media().getAttribute("src")).not.toBeNull());
    await userEvent.click(screen.getByRole("button", { name: es.assistant.playVoiceNote }));
    expect(played).toHaveBeenCalledTimes(1);
    fireEvent.play(media());
    await userEvent.click(screen.getByRole("button", { name: es.assistant.stopVoiceNote }));
    expect(paused).toHaveBeenCalledTimes(1);
    expect(media().currentTime).toBe(0);
    fireEvent.pause(media());
    expect(screen.getByRole("button", { name: es.assistant.playVoiceNote })).toBeTruthy();
    played.mockRestore();
    paused.mockRestore();
  });
});
