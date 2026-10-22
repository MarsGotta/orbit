import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { WriteOutcome } from "../../../bridge/generated/WriteOutcome";
import { ToastStack } from "../../../design/Toast";
import { UnseenWriteOutcomes } from "./UnseenWriteOutcomes";

// A calendar write whose outcome arrives while the calendar view is closed
// (a move or a save just before leaving it) is said anyway, as the open view
// says it in its panel (`notices.ts`, `failureNotice`).

const refused: WriteOutcome = {
  request_id: "escritura-1",
  operation: "update",
  stage: "refused",
  code: "calendar_not_editable",
  notice: "El calendario «Trabajo» no admite cambios desde este Mac.",
};

afterEach(cleanup);

function shown(writes: WriteOutcome[], calendarOpen: boolean, onOpen = vi.fn()) {
  const view = render(
    <>
      <ToastStack />
      <UnseenWriteOutcomes calendarOpen={calendarOpen} onOpenCalendar={onOpen} writes={writes} />
    </>,
  );
  const rerender = (next: WriteOutcome[], open: boolean) =>
    view.rerender(
      <>
        <ToastStack />
        <UnseenWriteOutcomes calendarOpen={open} onOpenCalendar={onOpen} writes={next} />
      </>,
    );
  return { rerender, onOpen };
}

describe("a calendar write's outcome with its view closed", () => {
  it("a refused write is said, and the calendar is one click away", () => {
    const { rerender, onOpen } = shown([], false);
    rerender([refused], false);
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("El calendario no aplicó el cambio");
    fireEvent.click(screen.getByRole("button", { name: "Abrir el calendario" }));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("an outcome nobody knows is said as unknown", () => {
    const { rerender } = shown([], false);
    rerender([{ request_id: "escritura-2", operation: "create", stage: "outcome_unknown" }], false);
    expect(screen.getByRole("alert").textContent).toContain("No se sabe si el cambio se guardó");
  });

  it("the open calendar says it itself: nothing is repeated here, then or later", () => {
    const { rerender } = shown([], true);
    rerender([refused], true);
    expect(screen.queryByRole("alert")).toBeNull();
    rerender([refused], false);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("a saved write says nothing, and a refusal is said once", () => {
    const { rerender } = shown([], false);
    rerender([{ request_id: "escritura-3", operation: "update", stage: "saved" }], false);
    expect(screen.queryByRole("alert")).toBeNull();
    rerender([refused], false);
    fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
    rerender([refused], false);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
