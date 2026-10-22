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

it("every notice, work in progress included, can be closed", () => {
  const dismiss = vi.fn();
  for (const kind of ["working", "done", "fixable", "failed"] as const) {
    const view = render(
      <NoticeBanner
        messages={[message(kind)]}
        onAction={vi.fn()}
        onDismiss={dismiss}
        onRetry={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
    expect(dismiss).toHaveBeenLastCalledWith(kind);
    view.unmount();
  }
});
