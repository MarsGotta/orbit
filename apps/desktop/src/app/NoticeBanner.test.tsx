import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { NoticeBanner } from "./NoticeBanner";
import type { Message, VisibleClass } from "./interceptor";

const message = (kind: VisibleClass, op = kind): Message => ({
  op,
  class: kind,
  text: `Aviso ${op}`,
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("todos los avisos, también el trabajo en curso, se pueden cerrar", () => {
  const dismiss = vi.fn();
  for (const kind of ["working", "done", "fixable", "failed"] as const) {
    const view = render(
      <NoticeBanner messages={[message(kind)]} onAction={vi.fn()} onDismiss={dismiss} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
    expect(dismiss).toHaveBeenLastCalledWith(kind);
    view.unmount();
  }
});
