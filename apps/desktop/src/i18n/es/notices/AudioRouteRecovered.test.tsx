import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { Notice } from "../../../bridge/generated/Notice";
import { receive } from "../../../app/interceptor";
import { NoticeBanner } from "../../../app/NoticeBanner";

function recovered(elapsedMs?: number) {
  const notice = {
    code: "audio_route_recovered",
    op: "audio-route-recovered",
    sequence: 1,
    data: elapsedMs === undefined ? {} : { elapsed_ms: elapsedMs },
  } as Notice;
  return receive([], { kind: "notice", notice }).messages;
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it.each<[number | undefined, string]>([
  [2_499, "Se perdió el audio 2 s al cambiar la ruta de audio; la grabación sigue."],
  [2_500, "Se perdió el audio 3 s al cambiar la ruta de audio; la grabación sigue."],
  [0, "Se perdió el audio menos de 1 s al cambiar la ruta de audio; la grabación sigue."],
  [999, "Se perdió el audio menos de 1 s al cambiar la ruta de audio; la grabación sigue."],
  [
    undefined,
    "Se perdió el audio al cambiar la ruta de audio; no se pudo medir la duración del hueco. La grabación sigue.",
  ],
])("explica el hueco con duración %s sin inventar la que falta", (duration, text) => {
  expect(recovered(duration)).toEqual([{ op: "audio-route-recovered", class: "done", text }]);
});

it("the visible notice has no action and expires after five seconds", () => {
  vi.useFakeTimers();
  const dismiss = vi.fn();
  render(<NoticeBanner messages={recovered(2_000)} onAction={vi.fn()} onDismiss={dismiss} />);
  expect(screen.getByRole("status").textContent).toContain("Se perdió el audio 2 s");
  expect(screen.getAllByRole("button")).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Cerrar aviso" })).toBeTruthy();
  act(() => vi.advanceTimersByTime(4_999));
  expect(screen.getByRole("status")).toBeTruthy();
  expect(dismiss).not.toHaveBeenCalled();
  act(() => vi.advanceTimersByTime(1));
  expect(screen.queryByRole("status")).toBeNull();
  expect(dismiss).toHaveBeenCalledExactlyOnceWith("audio-route-recovered");
});
