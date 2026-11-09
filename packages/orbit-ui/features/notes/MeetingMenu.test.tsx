import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { MeetingMenu } from "./MeetingMenu";
import { writtenNote } from "./test-notes";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

function menu(note: OpenNoteSnapshot = writtenNote()) {
  const send = vi.fn<(command: unknown) => Promise<boolean>>(() => Promise.resolve(true));
  render(<MeetingMenu note={note} send={send} />);
  return send;
}

async function ask() {
  await userEvent.click(screen.getByRole("button", { name: "Más" }));
  await userEvent.click(screen.getByRole("button", { name: "Borrar reunión" }));
}

describe("«Borrar reunión» in the note's menu", () => {
  it("only asks on the first click, saying what goes and where", async () => {
    const send = menu();
    await ask();
    expect(send).not.toHaveBeenCalled();
    const question = screen.getByRole("group", { name: "¿Borrar esta reunión?" });
    expect(question.textContent).toContain(
      "Se borran el audio, la transcripción y la nota, en este Mac y en el hub. No se puede deshacer.",
    );
    await userEvent.click(within(question).getByRole("button", { name: "Cancelar" }));
    expect(send).not.toHaveBeenCalled();
    expect(screen.queryByRole("group", { name: "¿Borrar esta reunión?" })).toBeNull();
  });

  it("deletes the meeting it was opened on and leaves it", async () => {
    window.location.hash = "#/notes/rec-1";
    const note = { ...writtenNote(), recording_id: "rec-1" };
    const send = menu(note);
    await ask();
    const question = screen.getByRole("group", { name: "¿Borrar esta reunión?" });
    await userEvent.click(within(question).getByRole("button", { name: "Borrar reunión" }));
    expect(send).toHaveBeenCalledWith({ command: "meeting_delete", recording_id: "rec-1" });
    await waitFor(() => expect(window.location.hash).toBe("#/notes"));
  });

  it("says a meeting still closing is deleted once it has closed", async () => {
    const closing: OpenNoteSnapshot = {
      ...writtenNote(),
      progress: {
        ...writtenNote().progress,
        upload: { state: "active", cause: null },
      },
    };
    menu(closing);
    await userEvent.click(screen.getByRole("button", { name: "Más" }));
    expect(screen.getByText("Se borrará al terminar de cerrarse")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Borrar reunión" }));
    const question = screen.getByRole("group", { name: "¿Borrar esta reunión?" });
    expect(question.textContent).toContain("Se borrará al terminar de cerrarse");
  });
});
