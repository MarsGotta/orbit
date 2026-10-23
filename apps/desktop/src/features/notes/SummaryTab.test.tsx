import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { flushBeforeQuit } from "../../state/quit-flush";
import { blur, editorNamed, focusAt, typeText } from "../editor/test-editor";
import { SummaryTab } from "./SummaryTab";
import { writtenNote } from "./test-notes";
import type { Rewriting } from "./use-rewriting";

// A section of the summary edited in place, in the notes' editor: what is
// sent is the Markdown as typed, a second after the typing stops or at once
// when the text loses the focus, and «✓ Guardado» only once the note says so.

/** The note is not being written again: «Rehacer resumen» can be pressed. */
const idle: Rewriting = { template: null, start: () => undefined, redo: () => undefined };

function edits(send: ReturnType<typeof vi.fn>): string[][] {
  return send.mock.calls
    .map(([command]) => command as AppCommand)
    .filter((command) => command.command === "note_edit")
    .map((command) =>
      command.command === "note_edit" ? command.sections.map((section) => section.markdown) : [],
    );
}

function withSection(note: OpenNoteSnapshot, index: number, markdown: string): OpenNoteSnapshot {
  const summary = note.summary;
  if (!summary) return note;
  return {
    ...note,
    summary: {
      ...summary,
      sections: summary.sections.map((section, at) =>
        at === index ? { ...section, markdown, edited: true } : section,
      ),
    },
  };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  vi.useRealTimers();
});

it("sends a section as typed, and says it is saved once the note carries it", async () => {
  const send = vi.fn(() => Promise.resolve(true));
  const note = writtenNote();
  const { rerender } = render(<SummaryTab note={note} rewriting={idle} send={send} />);
  fireEvent.click(screen.getByRole("button", { name: "Editar «Próximos pasos»" }));
  const editor = editorNamed("Texto de «Próximos pasos»");
  expect(editor.state.doc.toString()).toBe("1. Pedir las estanterías.");
  focusAt(editor);
  typeText(editor, "\nPedir  **dos** sillas");
  expect(screen.getByText("Guardando…")).toBeTruthy();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000);
  });
  const typed = "1. Pedir las estanterías.\n2. Pedir  **dos** sillas";
  expect(edits(send)).toEqual([["La sala abre en noviembre.", typed]]);
  rerender(<SummaryTab note={withSection(note, 1, typed)} rewriting={idle} send={send} />);
  expect(screen.getByText("✓ Guardado")).toBeTruthy();
});

it("says an edit the daemon kept is on this Mac until the hub has it, then that it is saved", async () => {
  const send = vi.fn(() => Promise.resolve(true));
  const note = writtenNote();
  const { rerender } = render(<SummaryTab note={note} rewriting={idle} send={send} />);
  fireEvent.click(screen.getByRole("button", { name: "Editar «Próximos pasos»" }));
  const editor = editorNamed("Texto de «Próximos pasos»");
  focusAt(editor);
  typeText(editor, "\nLlamar al proveedor");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000);
  });
  const typed = "1. Pedir las estanterías.\n2. Llamar al proveedor";
  // The daemon kept it and laid it over the note; the hub does not have it yet.
  const kept = { ...withSection(note, 1, typed), pending_changes: ["sections"] };
  rerender(<SummaryTab note={kept} rewriting={idle} send={send} />);
  expect(screen.getByText("Guardado en este Mac · el hub aún no lo tiene")).toBeTruthy();
  expect(screen.queryByText("✓ Guardado")).toBeNull();
  rerender(<SummaryTab note={withSection(note, 1, typed)} rewriting={idle} send={send} />);
  expect(screen.getByText("✓ Guardado")).toBeTruthy();
});

it("sends at once when the section's text loses the focus", () => {
  const send = vi.fn(() => Promise.resolve(true));
  render(<SummaryTab note={writtenNote()} rewriting={idle} send={send} />);
  fireEvent.click(screen.getByRole("button", { name: "Editar «Contexto»" }));
  const editor = editorNamed("Texto de «Contexto»");
  focusAt(editor);
  typeText(editor, " Con biblioteca.");
  blur(editor);
  expect(edits(send)).toEqual([
    ["La sala abre en noviembre. Con biblioteca.", "1. Pedir las estanterías."],
  ]);
});

describe("before Orbit quits, and after it fell", () => {
  const KEY = `orbit.draft.note.${writtenNote().recording_id}.summary`;

  it("an edit typed less than a second before quitting goes at once, and the quit waits for it", async () => {
    let take: (accepted: boolean) => void = () => undefined;
    const send = vi.fn(() => new Promise<boolean>((resolve) => (take = resolve)));
    render(<SummaryTab note={writtenNote()} rewriting={idle} send={send} />);
    fireEvent.click(screen.getByRole("button", { name: "Editar «Contexto»" }));
    const editor = editorNamed("Texto de «Contexto»");
    focusAt(editor);
    typeText(editor, " Abre a las nueve.");
    let done = false;
    const quitting = flushBeforeQuit().then(() => {
      done = true;
    });
    await act(async () => Promise.resolve());
    expect(edits(send)).toEqual([
      ["La sala abre en noviembre. Abre a las nueve.", "1. Pedir las estanterías."],
    ]);
    expect(done).toBe(false);
    await act(async () => take(true));
    await quitting;
    expect(done).toBe(true);
  });

  it("keeps the edit on this Mac until the daemon takes it, then lets it go", async () => {
    let take: (accepted: boolean) => void = () => undefined;
    const send = vi.fn(() => new Promise<boolean>((resolve) => (take = resolve)));
    render(<SummaryTab note={writtenNote()} rewriting={idle} send={send} />);
    fireEvent.click(screen.getByRole("button", { name: "Editar «Contexto»" }));
    const editor = editorNamed("Texto de «Contexto»");
    focusAt(editor);
    typeText(editor, "!");
    expect(JSON.parse(window.localStorage.getItem(KEY) ?? "null")).toMatchObject({
      edits: { "0": "La sala abre en noviembre.!" },
    });
    blur(editor);
    await act(async () => take(true));
    expect(window.localStorage.getItem(KEY)).toBeNull();
  });

  it("an edit not taken yet is back after Orbit closed or fell, says so, and is sent", async () => {
    const note = writtenNote();
    window.localStorage.setItem(
      KEY,
      JSON.stringify({
        version: note.summary?.generated_at_ms,
        edits: { "1": "1. Pedir las estanterías.\n2. Y dos sillas." },
      }),
    );
    vi.resetModules();
    const Reopened = (await import("./SummaryTab")).SummaryTab;
    const send = vi.fn(() => Promise.resolve(true));
    render(<Reopened note={note} rewriting={idle} send={send} />);
    expect(screen.getByText("Y dos sillas.")).toBeTruthy();
    expect(screen.getByText("Recuperado lo que escribiste y no se llegó a guardar.")).toBeTruthy();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(edits(send)).toEqual([
      ["La sala abre en noviembre.", "1. Pedir las estanterías.\n2. Y dos sillas."],
    ]);
  });

  it("an edit of a note the hub has written again since is not brought back", async () => {
    window.localStorage.setItem(KEY, JSON.stringify({ version: 1, edits: { "0": "vieja" } }));
    vi.resetModules();
    const Reopened = (await import("./SummaryTab")).SummaryTab;
    render(<Reopened note={writtenNote()} rewriting={idle} send={vi.fn()} />);
    expect(screen.queryByText("vieja")).toBeNull();
    expect(screen.queryByText(/Recuperado/)).toBeNull();
  });
});
