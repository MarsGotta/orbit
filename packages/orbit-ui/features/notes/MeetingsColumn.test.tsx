import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MeetingsColumn } from "./MeetingsColumn";
import { appView, notesSnapshot } from "./test-notes";
import { WithListColumn } from "./test-shell";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

function column(notes = notesSnapshot(), send = vi.fn(() => Promise.resolve())) {
  render(
    <WithListColumn>
      <MeetingsColumn send={send} snapshot={appView(notes)} />
    </WithListColumn>,
  );
  return send;
}

function deferred() {
  let resolve: () => void = () => undefined;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("the list of meetings", () => {
  it("groups the meetings by day, each with its title, time and one line", () => {
    column();
    const list = screen.getByRole("complementary", { name: "Lista de reuniones" });
    expect(within(list).getByRole("heading", { name: "Reuniones" })).toBeTruthy();
    const groups = within(list)
      .getAllByRole("heading", { level: 3 })
      .map((heading) => heading.textContent);
    expect(groups).toEqual(["Hoy", "Ayer", "Últimos 30 días", "Antes"]);
    const today = within(list).getByRole("link", { name: /Reunión de equipo/ });
    expect(today.textContent).toContain("9:00");
    expect(today.textContent).toContain("Bruno termina el cartel");
    const older = within(list).getByRole("link", { name: /Comité de la biblioteca/ });
    expect(older.textContent).toContain("14 oct");
    expect(within(list).getByRole("link", { name: /Llamada con Celia/ }).textContent).toContain(
      "Transcribiendo…",
    );
    expect(
      within(list).getByRole("link", { name: /Entrevistas a usuarios/ }).textContent,
    ).toContain("No se pudo escribir la nota");
  });

  it("marks the meeting that is open", () => {
    window.location.hash = "#/notes/rec-comite";
    column();
    const open = screen.getByRole("link", { name: /Comité de la biblioteca/ });
    expect(open.getAttribute("aria-current")).toBe("page");
    expect(open.getAttribute("href")).toBe("#/notes/rec-comite");
    expect(
      screen.getByRole("link", { name: /Reunión de equipo/ }).getAttribute("aria-current"),
    ).toBeNull();
  });

  it("opens a meeting and asks the daemon for its note", () => {
    const send = column();
    fireEvent.click(screen.getByRole("link", { name: /Reunión de equipo/ }));
    expect(window.location.hash).toBe("#/notes/rec-hoy");
    expect(send).toHaveBeenCalledWith({ command: "notes_open", recording_id: "rec-hoy" });
  });

  it("reads the list again with «Actualizar»", () => {
    const send = column();
    fireEvent.click(screen.getByRole("button", { name: "Actualizar" }));
    expect(send).toHaveBeenCalledWith({ command: "notes_refresh" });
  });

  it("says «Cargando…» while the next page is on its way, and then offers it again", async () => {
    const page = deferred();
    const send = vi.fn(() => page.promise);
    column(notesSnapshot({ index: { state: "available", more: true, detail: null } }), send);
    fireEvent.click(screen.getByRole("button", { name: "Cargar más reuniones" }));
    expect(send).toHaveBeenCalledWith({ command: "notes_more" });
    const more = screen.getByRole("button", { name: /Cargando…/ });
    expect(more.getAttribute("aria-busy")).toBe("true");
    await act(async () => {
      page.resolve();
      await page.promise;
    });
    expect(screen.getByRole("button", { name: "Cargar más reuniones" })).toBeTruthy();
  });

  it("while the list is read shows it loading, never as empty", () => {
    column(notesSnapshot({ list: [], index: undefined }));
    const list = screen.getByRole("complementary", { name: "Lista de reuniones" });
    expect(within(list).getByRole("status").textContent).toBe("Leyendo tus reuniones del hub…");
    expect(list.querySelector("[aria-busy='true']")).not.toBeNull();
    expect(screen.queryByText(/Aún no hay/)).toBeNull();
  });

  it("says when the list could not be read, with «Reintentar»", () => {
    const send = column(
      notesSnapshot({ list: [], index: { state: "failed", more: false, detail: "http 502" } }),
    );
    expect(screen.getByRole("alert").textContent).toContain(
      "No se pudo leer la lista de reuniones",
    );
    expect(screen.queryByText(/http 502/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(send).toHaveBeenCalledWith({ command: "notes_refresh" });
  });
});
