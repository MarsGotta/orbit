import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { receive } from "./interceptor";
import { NoticeBanner } from "./NoticeBanner";

afterEach(cleanup);
for (const [code, text, labels, commands] of [
  [
    "call_silence_question",
    "¿Sigues ahí? Si no dices nada, cuelgo en 15 segundos",
    ["Seguir", "Colgar"],
    ["call_keep", "call_stop"],
  ],
  [
    "recording_silence_question",
    "¿Sigue la reunión? La termino y la guardo en 30 s",
    ["Seguir grabando", "Terminar"],
    ["meeting_keep", "meeting_finish"],
  ],
] as const) {
  it(`${code} presents both choices and sends their orders`, () => {
    const op = code === "call_silence_question" ? "call-silence" : "recording-silence";
    const shown = receive([], { kind: "notice", notice: { code, op } });
    const onAction = vi.fn();
    render(
      <NoticeBanner
        messages={shown.messages}
        onAction={onAction}
        onDismiss={vi.fn()}
        onRetry={vi.fn()}
      />,
    );
    expect(screen.getByText(text)).toBeTruthy();
    labels.forEach((label, i) => {
      fireEvent.click(screen.getByRole("button", { name: label }));
      expect(onAction).toHaveBeenLastCalledWith({ label, command: { command: commands[i] } });
    });
    const resolved =
      code === "call_silence_question" ? "call_silence_resolved" : "recording_silence_resolved";
    expect(
      receive(shown.messages, { kind: "notice", notice: { code: resolved, op } }).messages,
    ).toEqual([]);
  });
}
