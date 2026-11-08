import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { withPendingVoice } from "./conversation";
import { MessageList } from "./MessageList";

afterEach(cleanup);

const question: ChatMessage = {
  id: "pregunta-sintetica",
  role: "user",
  text: "Una pregunta sintética",
  at_ms: 1,
  streaming: false,
  voice: null,
  receipts: [],
};

const waiting: ChatMessage = {
  id: "nota-sintetica",
  role: "user",
  text: "",
  at_ms: 2,
  streaming: false,
  voice: {
    duration_ms: 4_000,
    failure: {
      code: "voice_hub_unreachable",
      provider_status: null,
      hub_status: null,
      timeout_ms: null,
      bytes: null,
      max_bytes: null,
    },
    audio_available: true,
  },
  receipts: [],
};

describe("the conversation with a voice note kept for the hub", () => {
  it("shows the note after the conversation, says it waits, and retries it", () => {
    const send = vi.fn<(command: AppCommand) => void>();
    render(<MessageList messages={withPendingVoice([question], [waiting])} send={send} />);
    expect(
      screen.getByText("Guardada en el Mac: se enviará sola cuando vuelva la conexión con el hub."),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Reintentar transcripción" }));
    expect(send).toHaveBeenCalledWith({ command: "voice_retry", message_id: "nota-sintetica" });
  });
});
