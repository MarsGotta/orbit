import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { blur, editorNamed, focusAt, typeText } from "../editor/test-editor";
import { SummaryTab } from "./SummaryTab";
import { writtenNote } from "./test-notes";

// A section of the summary edited in place, in the notes' editor: what is
// sent is the Markdown as typed, a second after the typing stops or at once
// when the text loses the focus, and «✓ Guardado» only once the note says so.

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
  const { rerender } = render(<SummaryTab note={note} send={send} />);
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
  rerender(<SummaryTab note={withSection(note, 1, typed)} send={send} />);
  expect(screen.getByText("✓ Guardado")).toBeTruthy();
});

it("sends at once when the section's text loses the focus", () => {
  const send = vi.fn(() => Promise.resolve(true));
  render(<SummaryTab note={writtenNote()} send={send} />);
  fireEvent.click(screen.getByRole("button", { name: "Editar «Contexto»" }));
  const editor = editorNamed("Texto de «Contexto»");
  focusAt(editor);
  typeText(editor, " Con biblioteca.");
  blur(editor);
  expect(edits(send)).toEqual([
    ["La sala abre en noviembre. Con biblioteca.", "1. Pedir las estanterías."],
  ]);
});
