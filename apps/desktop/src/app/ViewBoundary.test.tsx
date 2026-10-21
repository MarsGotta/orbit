import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { genericFailure, receive } from "./interceptor";
import { ViewBoundary } from "./ViewBoundary";

const OP = "c0ffee00-0000-4000-8000-000000000003";

function Broken(): never {
  throw new Error("a view that does not paint");
}

describe("a view that fails to paint", () => {
  it("is caught by its boundary and shown as the generic failure, never a blank screen", () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const crash = vi.fn(
      () =>
        receive([], { kind: "crash", op: OP, code: "ui_render_failed" }).messages[0]?.text ?? "",
    );
    render(
      <ViewBoundary onCrash={crash}>
        <Broken />
      </ViewBoundary>,
    );
    expect(crash).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("alert").textContent).toBe(genericFailure(OP));
  });
});
