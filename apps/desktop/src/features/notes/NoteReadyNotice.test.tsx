import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NoteReadySnapshot } from "../../bridge/generated/NoteReadySnapshot";
import { NoteReadyNotice } from "./NoteReadyNotice";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

const ready = (sequence: number, title = "Comité de la biblioteca"): NoteReadySnapshot => ({
  recording_id: `rec-${sequence}`,
  title,
  sequence,
});

describe("«Tu nota está lista»", () => {
  it("says nothing for a note that was ready before the window looked", () => {
    render(<NoteReadyNotice ready={ready(4)} send={vi.fn()} />);
    expect(screen.queryByText("Tu nota está lista")).toBeNull();
  });

  it("tells each note that becomes ready once, with its title and «Abrir»", () => {
    const send = vi.fn();
    const view = render(<NoteReadyNotice ready={ready(4)} send={send} />);
    view.rerender(<NoteReadyNotice ready={ready(5, "Reunión de equipo")} send={send} />);
    expect(screen.getByText("Tu nota está lista")).toBeTruthy();
    expect(screen.getByText("Reunión de equipo")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Abrir" }));
    expect(window.location.hash).toBe("#/notes/rec-5");
    expect(send).toHaveBeenCalledWith({ command: "notes_open", recording_id: "rec-5" });
    expect(screen.queryByText("Tu nota está lista")).toBeNull();
  });

  it("tells the first note ready when none was before", () => {
    const view = render(<NoteReadyNotice ready={undefined} send={vi.fn()} />);
    expect(screen.queryByText("Tu nota está lista")).toBeNull();
    view.rerender(<NoteReadyNotice ready={ready(1)} send={vi.fn()} />);
    expect(screen.getByText("Tu nota está lista")).toBeTruthy();
  });
});
