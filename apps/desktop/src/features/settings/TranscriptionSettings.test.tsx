import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentPort } from "./agents/use-agent-config";
import { TranscriptionSettings } from "./TranscriptionSettings";

afterEach(cleanup);

it("reads the hub choice, explains the transfer, validates and forgets the key", async () => {
  let model = "whisper";
  const agent = vi.fn(async (call) => {
    if (call.call === "transcription_save") model = call.model;
    return {
      call,
      outcome: {
        outcome: "transcription_settings",
        settings: {
          model,
          key_configured: model === "mai",
        },
      },
    };
  }) as AgentPort;
  render(<TranscriptionSettings agent={agent} />);
  const select = await screen.findByRole("combobox", { name: "Modelo de transcripción" });
  expect((select as HTMLSelectElement).value).toBe("whisper");
  fireEvent.change(select, { target: { value: "mai" } });
  expect(screen.getByText(/Microsoft a través de OpenRouter/)).toBeTruthy();
  const key = screen.getByLabelText("Clave de OpenRouter");
  expect(key.getAttribute("type")).toBe("password");
  fireEvent.change(key, { target: { value: "invented-test-key" } });
  fireEvent.click(screen.getByRole("button", { name: "Guardar transcripción" }));
  await screen.findByText("Guardado en el hub");
  expect(agent).toHaveBeenCalledWith({
    call: "transcription_save",
    model: "mai",
    api_key: "invented-test-key",
  });
  expect((key as HTMLInputElement).value).toBe("");
});

it("keeps a refusal in place without claiming the change was saved", async () => {
  const agent = vi.fn(async (call) => ({
    call,
    outcome:
      call.call === "transcription_settings"
        ? {
            outcome: "transcription_settings",
            settings: { model: "whisper", key_configured: false },
          }
        : {
            outcome: "refused",
            refusal: {
              code: "transcription_quota_exhausted",
              http_status: 502,
              numbers: { provider_status: 402 },
              message: "Se ha agotado el saldo de OpenRouter",
            },
          },
  })) as AgentPort;
  render(<TranscriptionSettings agent={agent} />);
  fireEvent.change(await screen.findByRole("combobox"), { target: { value: "mai" } });
  fireEvent.change(screen.getByLabelText("Clave de OpenRouter"), { target: { value: "invented" } });
  fireEvent.click(screen.getByRole("button", { name: "Guardar transcripción" }));
  await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("saldo"));
  expect(screen.queryByText("Guardado en el hub")).toBeNull();
});
