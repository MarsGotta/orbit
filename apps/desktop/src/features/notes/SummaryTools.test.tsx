import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { NoteDetail } from "./NoteDetail";
import { library, writtenNote } from "./test-notes";

// «Rehacer resumen» moved from the note's header to the Resumen's toolbar
// (annex, change 7), with its own question first.

afterEach(cleanup);

function written(): OpenNoteSnapshot {
  return { ...writtenNote(), recording_id: "meeting-resumen", title: "Comité de presupuesto" };
}

function detail(note: OpenNoteSnapshot, send = vi.fn(() => Promise.resolve(true))) {
  render(<NoteDetail error={null} library={library()} note={note} send={send} />);
  return send;
}

describe("«Rehacer resumen» in the Resumen", () => {
  it("sits beside «Copiar resumen» and asks first, saying the hand-made version is kept", async () => {
    const send = detail(written());
    fireEvent.click(screen.getByRole("button", { name: "Rehacer resumen" }));
    const question = screen.getByRole("dialog", { name: "¿Rehacer el resumen?" });
    expect(question.textContent).toContain(
      "Orbit lo escribe de nuevo con la plantilla Revisión y la transcripción actual. Si lo cambiaste a mano, tu versión queda guardada.",
    );
    expect(send).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(within(question).getByRole("button", { name: "Rehacer resumen" }));
    });
    expect(send).toHaveBeenCalledExactlyOnceWith({
      command: "note_regenerate",
      recording_id: "meeting-resumen",
      template_id: "review",
    });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("says while it writes it, under the header, and cannot be pressed again then", () => {
    detail(
      written(),
      vi.fn(() => new Promise<boolean>(() => undefined)),
    );
    fireEvent.click(screen.getByRole("button", { name: "Rehacer resumen" }));
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Rehacer resumen" }),
    );
    expect(
      screen.getByText("Reescribiendo la nota con «Revisión». Tu versión anterior queda guardada."),
    ).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Rehacer resumen" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("waits for the recording to end, and says so", () => {
    const note = written();
    note.progress.upload = { state: "active", cause: { kind: "recording" } };
    detail(note);
    fireEvent.click(screen.getByRole("tab", { name: /^Resumen/ }));
    const button = screen.getByRole("button", { name: "Rehacer resumen" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(
      screen.getAllByText("La nota se escribe al terminar la grabación.").length,
    ).toBeGreaterThan(0);
  });

  it("writes a note that was never asked for", () => {
    const note = written();
    note.summary = null;
    note.progress.summary = { state: "waiting", cause: { kind: "not_requested" } };
    const send = detail(note);
    fireEvent.click(screen.getByRole("tab", { name: /^Resumen/ }));
    expect(
      screen.getByText("Esta reunión aún no tiene nota. Pulsa «Rehacer resumen» para escribirla."),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Rehacer resumen" }));
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Rehacer resumen" }),
    );
    expect(send).toHaveBeenCalledWith(expect.objectContaining({ command: "note_regenerate" }));
  });
});

describe("«Rehacer resumen» from a band under the transcript", () => {
  it("writes the note again and shows the Resumen", async () => {
    const note = written();
    const send = detail(note);
    fireEvent.click(screen.getByRole("tab", { name: "Transcripción" }));
    fireEvent.click(screen.getAllByRole("button", { name: /^Alina/ })[0]!);
    fireEvent.change(screen.getByRole("textbox", { name: "Buscar o añadir un nombre" }), {
      target: { value: "Ada" },
    });
    fireEvent.click(screen.getByRole("button", { name: "+ Añadir «Ada»" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Aplicar" }));
    });
    await act(async () => {
      fireEvent.click(
        within(screen.getByText(/Cambiado en/).closest("p")!).getByRole("button", {
          name: "Rehacer resumen",
        }),
      );
    });
    expect(send).toHaveBeenLastCalledWith({
      command: "note_regenerate",
      recording_id: "meeting-resumen",
      template_id: "review",
    });
    expect(screen.getByRole("tab", { name: /^Resumen/ }).getAttribute("aria-selected")).toBe(
      "true",
    );
  });
});
