import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { DaemonPort } from "../../bridge/port";
import { PanelApp } from "./PanelApp";

// The floating panel follows the same frames as the window. When it cannot
// connect it says so, as the window does, instead of staying blank.

describe("the floating panel's connection", () => {
  it("says it could not connect", async () => {
    const port: DaemonPort = {
      subscribeFrames: async () => () => undefined,
      snapshot: () => Promise.reject(new Error("sin arranque en esta prueba")),
      send: vi.fn(),
      report: vi.fn(async () => undefined),
    };
    render(<PanelApp port={port} />);
    expect(
      await screen.findByText("No se pudo conectar con Orbit. Ciérralo y vuelve a abrirlo."),
    ).toBeTruthy();
  });
});
