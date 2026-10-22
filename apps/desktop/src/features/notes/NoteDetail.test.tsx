import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { NoteDetail } from "./NoteDetail";
import { library, writingNote, writtenNote } from "./test-notes";

afterEach(cleanup);

function detail(
  note: OpenNoteSnapshot,
  send = vi.fn<(command: unknown) => Promise<void>>(() => Promise.resolve()),
) {
  const view = render(<NoteDetail error={null} library={library()} note={note} send={send} />);
  return { send, view };
}

function tabs() {
  return screen.getAllByRole("tab").map((tab) => tab.textContent);
}

describe("the parts of a written note", () => {
  it("opens on the summary, with the open tasks counted in their tab", () => {
    detail(writtenNote());
    expect(tabs()).toEqual(["Resumen", "Acciones · 3", "Transcripción", "Notas privadas"]);
    expect(screen.getByRole("tab", { name: "Resumen" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.queryByText("Orbit está preparando tu nota")).toBeNull();
  });

  it("shows each section, the decisions, the open topics and who spoke how much", () => {
    detail(writtenNote());
    const panel = screen.getByRole("tabpanel");
    for (const name of [
      "Contexto",
      "Próximos pasos",
      "Decisiones",
      "Temas abiertos",
      "Quién habló",
    ])
      expect(within(panel).getByRole("heading", { name })).toBeTruthy();
    expect(within(panel).getByText("Alina 25 %")).toBeTruthy();
    expect(within(panel).getByText("Persona 2 50 %")).toBeTruthy();
  });
});

describe("every «Copiar» says it copied", () => {
  const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
  beforeEach(() => {
    writeText.mockClear();
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  });

  it.each([
    ["Resumen", "Copiar resumen"],
    ["Acciones · 3", "Copiar todas"],
    ["Transcripción", "Copiar"],
  ])("in %s, «%s» → «✓ Copiado»", async (tab, label) => {
    detail(writtenNote());
    fireEvent.click(screen.getByRole("tab", { name: tab }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: label }));
    });
    expect(writeText).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "✓ Copiado" })).toBeTruthy();
  });
});

describe("the tasks", () => {
  it("are grouped by person, yours first, with how many are left", () => {
    detail(writtenNote());
    fireEvent.click(screen.getByRole("tab", { name: "Acciones · 3" }));
    const panel = screen.getByRole("tabpanel");
    expect(within(panel).getByText("Quedan 3 tareas")).toBeTruthy();
    expect(
      within(panel)
        .getAllByRole("heading", { level: 3 })
        .map((heading) => heading.textContent),
    ).toEqual(["Tú", "Bruno Lago", "Sin asignar"]);
    expect(within(panel).getByRole("checkbox", { name: /Reservar la sala/ }).textContent).toContain(
      "para el viernes",
    );
  });

  it("say «Guardando…» once marked, until the hub has the mark", () => {
    const { send, view } = detail(writtenNote());
    fireEvent.click(screen.getByRole("tab", { name: "Acciones · 3" }));
    fireEvent.click(screen.getByRole("checkbox", { name: /Pedir tres presupuestos/ }));
    expect(send).toHaveBeenCalledWith({
      command: "note_action_toggle",
      recording_id: "rec-comite",
      action_id: "a1",
      done: true,
    });
    const box = screen.getByRole("checkbox", { name: /Pedir tres presupuestos/ });
    expect(box.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("Guardando…")).toBeTruthy();
    const marked = writtenNote();
    marked.actions = marked.actions.map((action) =>
      action.id === "a1" ? { ...action, done: true } : action,
    );
    view.rerender(<NoteDetail error={null} library={library()} note={marked} send={send} />);
    expect(screen.queryByText("Guardando…")).toBeNull();
    expect(screen.getByRole("tab", { name: "Acciones · 2" })).toBeTruthy();
  });
});

describe("a note being written", () => {
  it("shows the three steps and opens on the transcript as it fills", () => {
    detail(writingNote());
    const card = screen.getByRole("region", { name: "Orbit está preparando tu nota" });
    expect(
      within(card).getByText(
        "Suele tardar menos de un minuto. Puedes seguir con lo tuyo; te avisaremos cuando esté lista.",
      ),
    ).toBeTruthy();
    expect(
      within(card)
        .getAllByRole("listitem")
        .map((step) => step.textContent),
    ).toEqual(["Audio guardado", "Transcribiendo…", "Escribir el resumen"]);
    expect(tabs()).toEqual(["Resumen", "Acciones", "Transcripción", "Notas privadas"]);
    expect(screen.getByRole("tab", { name: "Transcripción" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(
      screen.getByText(
        "La transcripción va apareciendo. Pulsa un nombre para cambiar quién habla.",
      ),
    ).toBeTruthy();
    expect(screen.getByLabelText("Llegando más transcripción")).toBeTruthy();
  });

  it("says the summary is on its way once the transcript is done", () => {
    const note = writingNote();
    note.progress.transcript = { state: "done", cause: null };
    note.progress.summary = { state: "active", cause: { kind: "writing" } };
    detail(note);
    const card = screen.getByRole("region", { name: "Orbit está preparando tu nota" });
    expect(
      within(card)
        .getAllByRole("listitem")
        .map((step) => step.textContent),
    ).toEqual(["Audio guardado", "Transcripción lista", "Escribiendo el resumen…"]);
    fireEvent.click(screen.getByRole("tab", { name: "Resumen" }));
    expect(
      screen.getByText(
        "El resumen aparecerá aquí en cuanto se escriba, con las secciones de tu plantilla «Revisión».",
      ),
    ).toBeTruthy();
  });
});

describe("a note that could not be written", () => {
  it("says so, why, and offers «Reintentar» with the series' template", () => {
    const note = writingNote();
    note.progress.transcript = { state: "done", cause: null };
    note.progress.summary = { state: "failed", cause: { kind: "failed" } };
    note.transcript.pending_chunks = 0;
    const { send } = detail(note);
    const alert = screen.getByRole("alert");
    expect(within(alert).getByText("No se pudo escribir la nota")).toBeTruthy();
    expect(alert.textContent).toContain("El hub no pudo escribirla.");
    fireEvent.click(within(alert).getByRole("button", { name: "Reintentar" }));
    expect(send).toHaveBeenCalledWith({
      command: "note_regenerate",
      recording_id: "rec-comite",
      template_id: "review",
    });
    expect(screen.queryByText("Orbit está preparando tu nota")).toBeNull();
  });

  it("names a hub without a model to write notes", () => {
    const note = writingNote();
    note.progress.summary = { state: "failed", cause: { kind: "not_configured" } };
    detail(note);
    expect(screen.getByRole("alert").textContent).toContain(
      "El hub no tiene un modelo para escribir notas.",
    );
  });

  it("retries the part of the transcript the hub gave up on", () => {
    const note = writingNote();
    note.progress.transcript = { state: "failed", cause: { kind: "transcript_failed", groups: 2 } };
    note.transcript.pending_chunks = 0;
    note.transcript.failed_groups = 2;
    const { send } = detail(note);
    const alert = screen.getByRole("alert");
    expect(within(alert).getByText("No se pudo transcribir toda la reunión")).toBeTruthy();
    fireEvent.click(within(alert).getByRole("button", { name: "Reintentar" }));
    expect(send).toHaveBeenCalledWith({ command: "transcript_retry", recording_id: "rec-comite" });
  });
});

describe("the meeting being recorded", () => {
  it("opens on the private notes and shows no progress card", () => {
    const note = writingNote();
    note.capture = "open";
    note.progress.upload = { state: "active", cause: { kind: "recording" } };
    detail(note);
    expect(screen.getByRole("tab", { name: "Notas privadas" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(screen.queryByText("Orbit está preparando tu nota")).toBeNull();
  });
});
