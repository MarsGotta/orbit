import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { blur, editorNamed, focusAt, typeText } from "../editor/test-editor";
import { PrivateNotes } from "./PrivateNotes";
import { writtenNote } from "./test-notes";

// Alina's private notes, typed in the note's editor: what is saved is what
// she typed, a second after she stops, at once when she leaves the text or
// the note, and nothing she typed is lost when a save is refused.

function saves(send: ReturnType<typeof vi.fn>): string[] {
  return send.mock.calls
    .map(([command]) => command as AppCommand)
    .filter((command) => command.command === "note_private_save")
    .map((command) => (command.command === "note_private_save" ? command.markdown : ""));
}

function note(privateNotes = ""): OpenNoteSnapshot {
  return { ...writtenNote(), private_notes: privateNotes };
}

function renderNotes(send: ReturnType<typeof vi.fn>, stored = "", error: string | null = null) {
  const view = render(<PrivateNotes error={error} note={note(stored)} send={send} />);
  return { ...view, editor: editorNamed("Notas privadas") };
}

/** Lets the timers run on, and the promises they start settle. */
async function wait(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("saves the Markdown exactly as typed, a second after the typing stops", async () => {
  const send = vi.fn(() => Promise.resolve(true));
  const { editor } = renderNotes(send);
  focusAt(editor);
  // Two spaces, a star that is not a mark and a numbered list starting at 3.
  const typed = "## Para el comité\nConfirmar con  *Bruno* la sala 2*3\n3. primero";
  typeText(editor, typed);
  await wait(900);
  expect(saves(send)).toEqual([]);
  await wait(200);
  expect(saves(send)).toEqual([typed]);
});

it("says «Guardando…» until the daemon kept it, and then that it is on this Mac", async () => {
  let keep: (accepted: boolean) => void = () => undefined;
  const send = vi.fn(() => new Promise<boolean>((resolve) => (keep = resolve)));
  const { editor } = renderNotes(send);
  expect(screen.getByText("Guardado en este Mac · solo lo ves tú")).toBeTruthy();
  focusAt(editor);
  typeText(editor, "Sala norte");
  expect(screen.getByText("Guardando…")).toBeTruthy();
  await wait(1_000);
  expect(saves(send)).toEqual(["Sala norte"]);
  expect(screen.getByText("Guardando…")).toBeTruthy();
  await act(async () => keep(true));
  expect(screen.getByText("Guardado en este Mac · solo lo ves tú")).toBeTruthy();
});

it("saves at once when the text loses the focus, and when the note is left", () => {
  const send = vi.fn(() => Promise.resolve(true));
  const { editor, unmount } = renderNotes(send);
  focusAt(editor);
  typeText(editor, "uno");
  blur(editor);
  expect(saves(send)).toEqual(["uno"]);
  focusAt(editor);
  typeText(editor, " dos");
  unmount();
  expect(saves(send)).toEqual(["uno", "uno dos"]);
});

it("a refused save keeps the text, says why, and goes again before the note is left", async () => {
  const send = vi.fn(() => Promise.resolve(false));
  const { editor, rerender, unmount } = renderNotes(send);
  focusAt(editor);
  typeText(editor, "Llamar a Celia");
  await wait(1_000);
  rerender(<PrivateNotes error="El disco de este Mac está lleno." note={note()} send={send} />);
  expect(editor.state.doc.toString()).toBe("Llamar a Celia");
  expect(
    screen.getByText("No se ha guardado: El disco de este Mac está lleno. Lo escrito sigue aquí."),
  ).toBeTruthy();
  send.mockImplementation(() => Promise.resolve(true));
  unmount();
  expect(saves(send)).toEqual(["Llamar a Celia", "Llamar a Celia"]);
});

it("an echo of an earlier save does not replace what was typed since", async () => {
  const send = vi.fn(() => Promise.resolve(true));
  const { editor, rerender } = renderNotes(send, "a");
  focusAt(editor);
  typeText(editor, "b");
  await wait(1_000);
  typeText(editor, "c");
  blur(editor);
  rerender(<PrivateNotes error={null} note={note("ab")} send={send} />);
  expect(editor.state.doc.toString()).toBe("abc");
  rerender(<PrivateNotes error={null} note={note("abc")} send={send} />);
  rerender(<PrivateNotes error={null} note={note("abcd")} send={send} />);
  // A newer text written elsewhere, once nothing here is waiting, is shown.
  expect(editor.state.doc.toString()).toBe("abcd");
});

it("says how much is left before the note reaches 65.536 characters", () => {
  const send = vi.fn(() => Promise.resolve(true));
  // 6554 left: more than a tenth of the ceiling, so nothing is said yet.
  const { editor } = renderNotes(send, "a".repeat(58_982));
  expect(screen.queryByText(/Quedan/)).toBeNull();
  focusAt(editor);
  typeText(editor, "b");
  expect(screen.getByText("Quedan 6553 de 65.536 caracteres.")).toBeTruthy();
});
