import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { flushBeforeQuit } from "../../state/quit-flush";
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

describe("before Orbit quits, and after it fell", () => {
  it("a note typed less than a second before quitting goes at once, and the quit waits for it", async () => {
    let keep: (accepted: boolean) => void = () => undefined;
    const send = vi.fn(() => new Promise<boolean>((resolve) => (keep = resolve)));
    const { editor } = renderNotes(send);
    focusAt(editor);
    typeText(editor, "Sala norte");
    let done = false;
    const quitting = flushBeforeQuit().then(() => {
      done = true;
    });
    await act(async () => Promise.resolve());
    expect(saves(send)).toEqual(["Sala norte"]);
    expect(done).toBe(false);
    await act(async () => keep(true));
    await quitting;
    expect(done).toBe(true);
  });

  it("what was typed and not taken yet is back after Orbit closed or fell, says so, and is saved", async () => {
    // What an earlier run left on this Mac: typed over the note as stored.
    window.localStorage.setItem(
      `orbit.draft.note.${writtenNote().recording_id}.private`,
      JSON.stringify({ from: "", text: "Llamar a Celia antes del jueves" }),
    );
    vi.resetModules();
    const Reopened = (await import("./PrivateNotes")).PrivateNotes;
    const send = vi.fn(() => Promise.resolve(true));
    render(<Reopened error={null} note={note()} send={send} />);
    expect(screen.getByRole("textbox", { name: "Notas privadas" }).textContent).toBe(
      "Llamar a Celia antes del jueves",
    );
    expect(screen.getByText("Recuperado lo que escribiste y no se llegó a guardar.")).toBeTruthy();
    await wait(1_000);
    expect(saves(send)).toEqual(["Llamar a Celia antes del jueves"]);
    expect(screen.getByText("Guardado en este Mac · solo lo ves tú")).toBeTruthy();
  });

  it("keeps what is typed on this Mac until the daemon takes it, then lets it go", async () => {
    const key = `orbit.draft.note.${writtenNote().recording_id}.private`;
    let keep: (accepted: boolean) => void = () => undefined;
    const send = vi.fn(() => new Promise<boolean>((resolve) => (keep = resolve)));
    const { editor } = renderNotes(send);
    focusAt(editor);
    typeText(editor, "Sala");
    expect(JSON.parse(window.localStorage.getItem(key) ?? "null")).toEqual({
      from: "",
      text: "Sala",
    });
    await wait(1_000);
    await act(async () => keep(true));
    expect(window.localStorage.getItem(key)).toBeNull();
  });

  it("a draft written over another text than the one stored now is not brought back", async () => {
    window.localStorage.setItem(
      `orbit.draft.note.${writtenNote().recording_id}.private`,
      JSON.stringify({ from: "otra versión", text: "vieja" }),
    );
    vi.resetModules();
    const Reopened = (await import("./PrivateNotes")).PrivateNotes;
    render(<Reopened error={null} note={note("la de ahora")} send={vi.fn()} />);
    expect(screen.getByRole("textbox", { name: "Notas privadas" }).textContent).toBe("la de ahora");
  });
});
