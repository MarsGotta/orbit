import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NotesSnapshot } from "../../bridge/generated/NotesSnapshot";
import NotesView from "./NotesView";
import { appView, notesSnapshot, writtenNote } from "./test-notes";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

function view(notes: NotesSnapshot, onRecord = vi.fn()) {
  const send = vi.fn(() => Promise.resolve());
  render(<NotesView error={null} onRecord={onRecord} send={send} snapshot={appView(notes)} />);
  return { send, onRecord };
}

describe("Reuniones before any meeting", () => {
  it("invites to record the first one", () => {
    const { onRecord } = view(notesSnapshot({ list: [] }));
    expect(screen.getByRole("heading", { name: "Aún no hay reuniones" })).toBeTruthy();
    expect(
      screen.getByText("Graba tu primera reunión y Orbit escribirá la nota por ti."),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Grabar reunión" }));
    expect(onRecord).toHaveBeenCalledOnce();
  });

  it("never says there are none while the list is read", () => {
    view(notesSnapshot({ list: [], index: { state: "loading", more: false, detail: null } }));
    expect(screen.getByRole("status").textContent).toBe("Leyendo tus reuniones del hub…");
    expect(screen.queryByText(/Aún no hay/)).toBeNull();
  });

  it("never says there are none before the list was read at all", () => {
    view(notesSnapshot({ list: [], index: undefined }));
    expect(screen.queryByText(/Aún no hay/)).toBeNull();
    expect(screen.getByRole("status").textContent).toBe("Leyendo tus reuniones del hub…");
  });
});

describe("a meeting being opened", () => {
  it("shows the note loading while it is read", () => {
    window.location.hash = "#/notes/rec-comite";
    const { send } = view(notesSnapshot());
    expect(send).toHaveBeenCalledWith({ command: "notes_open", recording_id: "rec-comite" });
    expect(screen.getByRole("status").textContent).toBe("Leyendo la nota del hub…");
    expect(document.querySelector("[aria-busy='true']")).not.toBeNull();
  });

  it("says why a note could not be opened, with «Reintentar»", () => {
    window.location.hash = "#/notes/rec-comite";
    const { send } = view(
      notesSnapshot({
        open: writtenNote(),
        open_read: {
          recording_id: "rec-comite",
          revision: 1,
          state: "failed",
          detail: "hub_unreachable",
          read_at_ms: null,
        },
      }),
    );
    expect(screen.getByRole("alert").textContent).toContain("No se pudo abrir la nota");
    expect(screen.queryByText(/hub_unreachable/)).toBeNull();
    send.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(send).toHaveBeenCalledWith({ command: "notes_open", recording_id: "rec-comite" });
  });

  it("shows the note once it is read", () => {
    window.location.hash = "#/notes/rec-comite";
    view(
      notesSnapshot({
        open: writtenNote(),
        open_read: {
          recording_id: "rec-comite",
          revision: 3,
          state: "available",
          detail: null,
          read_at_ms: 1,
        },
      }),
    );
    expect(
      (screen.getByRole("textbox", { name: "Título de la reunión" }) as HTMLInputElement).value,
    ).toBe("Comité de la biblioteca");
  });

  it("asks to choose one when meetings are listed and none is open", () => {
    view(notesSnapshot());
    expect(screen.getByRole("heading", { name: "Elige una reunión" })).toBeTruthy();
  });

  it("says a meeting that no longer exists, without «Reintentar» or reading it again", () => {
    window.location.hash = "#/notes/rec-comite";
    const { send } = view(
      notesSnapshot({
        open: null,
        open_read: {
          recording_id: "rec-comite",
          revision: 4,
          state: "gone",
          detail: null,
          read_at_ms: null,
        },
      }),
    );
    expect(screen.getByRole("heading", { name: "Esta reunión ya no existe" })).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("button", { name: "Reintentar" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Título de la reunión" })).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("leaves a deleted meeting for the list, with no note selected", () => {
    window.location.hash = "#/notes";
    const { send } = view(
      notesSnapshot({
        open: null,
        open_read: {
          recording_id: "rec-comite",
          revision: 4,
          state: "gone",
          detail: null,
          read_at_ms: null,
        },
      }),
    );
    expect(screen.getByRole("heading", { name: "Elige una reunión" })).toBeTruthy();
    expect(screen.queryByText("Esta reunión ya no existe")).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Título de la reunión" })).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("leaves a meeting being deleted for the choice of another", () => {
    view(notesSnapshot({ open: { ...writtenNote(), pending_changes: ["delete"] } }));
    expect(screen.queryByRole("textbox", { name: "Título de la reunión" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Elige una reunión" })).toBeTruthy();
  });
});
